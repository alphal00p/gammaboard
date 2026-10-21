import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from unittest.mock import Mock

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import ubelix as u
from gbops.deployment import Deployments, tag, job_name, write_json, owns
from gbops.slurm import Job, Slurm, SchedulerError, parse_jobs


@pytest.fixture
def workspace(tmp_path, monkeypatch):
    monkeypatch.setattr(u, 'WORKSPACE_ROOT', str(tmp_path))
    (tmp_path / 'ops/config').mkdir(parents=True)
    for name in ('server.toml', 'server-single-node.toml'):
        shutil.copy2(ROOT / 'config' / name, tmp_path / 'ops/config' / name)
    shutil.copy2(ROOT / 'ubelix.py', tmp_path / 'ubelix.py')
    shutil.copytree(ROOT / 'gbops', tmp_path / 'gbops', ignore=shutil.ignore_patterns('__pycache__'))
    shutil.copytree(ROOT / 'slurm', tmp_path / 'ops/slurm')
    image = tmp_path / 'images/gammaboard/test.sif'
    image.parent.mkdir(parents=True)
    image.write_bytes(b'first immutable image')
    write_json(image.parent / 'current.json', {'path': str(image), 'sha256': hashlib.sha256(image.read_bytes()).hexdigest(),
                                              'build_info': {'build_id': 'test-build'}, 'validated': True})
    return tmp_path


class FakeSlurm:
    def __init__(self):
        self.jobs = {}
        self.submissions = []
        self.canceled = []
        self.interrupt_after_accept = False
        self.error = False

    def active(self):
        if self.error: raise SchedulerError('scheduler unavailable')
        return [j for j in self.jobs.values() if not j.terminal]

    def lookup(self, identifier, since):
        if self.error: raise SchedulerError('scheduler unavailable')
        return self.jobs.get(identifier)

    def find(self, comment, since):
        if self.error: raise SchedulerError('accounting unavailable')
        return [j for j in self.jobs.values() if j.comment == comment]

    def submit(self, script, **kw):
        identifier = str(100 + len(self.submissions))
        self.submissions.append((script, kw))
        self.jobs[identifier] = Job(identifier, kw['name'], 'PENDING', '', kw['comment'])
        if self.interrupt_after_accept:
            self.interrupt_after_accept = False
            raise KeyboardInterrupt()
        return identifier

    def cancel(self, ids):
        self.canceled += ids
        for identifier in ids:
            j = self.jobs[identifier]
            self.jobs[identifier] = Job(j.id, j.name, 'CANCELLED', j.node, j.comment)


def create_manifest(workspace, offset=0):
    store = Deployments(workspace, offset)
    return store.create(store.selected_image(), store.configuration(False, 8080), False, False)


def control(manifest, scheduler, state='RUNNING'):
    j = Job('10', job_name(manifest, 'control'), state, 'compute-a' if state != 'PENDING' else '', tag(manifest, 'control'))
    scheduler.jobs[j.id] = j
    manifest['control_job_id'], manifest['control_submission'] = j.id, 'submitted'
    Deployments(manifest['workspace'], manifest['port_offset']).save(manifest)
    return j


def test_workspace_and_offset_are_separate_and_pending_down_never_calls_api(workspace, monkeypatch):
    scheduler = FakeSlurm()
    monkeypatch.setattr(u, 'wait_ready', lambda *_: ('compute-a', None))
    u.up(u.parser().parse_args(['up']), scheduler)
    first = Deployments(workspace, 0).load()
    u.up(u.parser().parse_args(['up', '--port-offset', '10']), scheduler)
    second = Deployments(workspace, 10).load()
    assert first['id'] != second['id']
    assert len(scheduler.submissions) == 2
    monkeypatch.setattr(u, 'Api', Mock(side_effect=AssertionError('pending jobs have no API')))
    u.down(u.parser().parse_args(['down', '--port-offset', '10']), scheduler)
    assert scheduler.canceled == [second['control_job_id']]
    assert not scheduler.jobs[first['control_job_id']].terminal
    other = workspace / 'other'
    other.mkdir()
    assert Deployments(other, 0).slot != Deployments(workspace, 0).slot


def test_control_submission_is_recovered_without_resubmitting(workspace, monkeypatch):
    scheduler = FakeSlurm()
    scheduler.interrupt_after_accept = True
    monkeypatch.setattr(u, 'wait_ready', lambda *_: ('compute-a', None))
    with pytest.raises(KeyboardInterrupt): u.up(u.parser().parse_args(['up']), scheduler)
    assert Deployments(workspace, 0).load()['control_submission'] == 'submitting'
    u.up(u.parser().parse_args(['up']), scheduler)
    assert len(scheduler.submissions) == 1
    assert Deployments(workspace, 0).load()['control_job_id'] == '100'


def test_unknown_control_or_scheduler_error_never_creates_replacement(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    manifest['control_submission'] = 'submitting'
    Deployments(workspace, 0).save(manifest)
    scheduler = FakeSlurm()
    with pytest.raises(RuntimeError, match='uncertain'): u.up(u.parser().parse_args(['up']), scheduler)
    scheduler.error = True
    with pytest.raises(SchedulerError): u.up(u.parser().parse_args(['up']), scheduler)
    assert not scheduler.submissions


def test_running_down_only_cancels_its_own_jobs(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs['20'] = Job('20', job_name(manifest, 'worker'), 'RUNNING', 'compute-b', tag(manifest, 'worker'))
    scheduler.jobs['30'] = Job('30', 'gb-other-control', 'RUNNING', 'compute-c', 'gb:other:control')
    api = Mock()
    monkeypatch.setattr(u, 'Api', lambda *_: api)
    u.down(u.parser().parse_args(['down', '--worker-timeout', '0', '--control-grace-seconds', '0']), scheduler)
    api.request.assert_called_once_with('/admin/workers/suspend')
    assert scheduler.canceled == ['20', '10']
    assert not scheduler.jobs['30'].terminal


def test_active_deployment_pins_image_and_rejects_changed_current_pointer(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    launcher = workspace / 'deployments' / manifest['id'] / 'launcher/ubelix.py'
    original = launcher.read_text()
    (workspace / 'ubelix.py').write_text('# changed by sync\n')
    assert launcher.read_text() == original
    scheduler = FakeSlurm(); control(manifest, scheduler)
    image = workspace / 'images/gammaboard/second.sif'; image.write_bytes(b'second version')
    write_json(image.parent / 'current.json', {'path': str(image), 'sha256': hashlib.sha256(image.read_bytes()).hexdigest(),
                                              'build_info': {'build_id': 'other-build'}, 'validated': True})
    with pytest.raises(RuntimeError, match='different image'): u.up(u.parser().parse_args(['up']), scheduler)
    command, env = u.container_command(manifest, ['gammaboard', 'build-info'])
    assert manifest['image']['path'] in command
    assert str(image) not in command
    assert env['APPTAINERENV_GAMMABOARD_DEPLOYMENT_ID'] == manifest['id']


def test_custom_tunnel_origin_and_private_configuration_are_snapshotted(workspace):
    (workspace / 'config').mkdir()
    private = workspace / 'config/server.toml'
    private.write_text('name="private"\n[auth]\nadmin_password_hash="hash"\nsession_secret="secret"\n')
    store = Deployments(workspace, 10)
    config = store.configuration(False, 9000)
    manifest = store.create(store.selected_image(), config, False, False)
    assert manifest['origins'] == ['http://localhost:9000', 'http://127.0.0.1:9000']
    assert 'private' in Path(manifest['server_config']).read_text()
    assert '9000:compute-a:8090' in u.tunnel_command('compute-a', 9000, 10)
    private.write_text('name="changed"')
    assert 'changed' not in Path(manifest['server_config']).read_text()
    Path(manifest['server_config']).write_text('name="tampered"')
    with pytest.raises(RuntimeError, match='snapshot'): u.container_command(manifest, [])
    with pytest.raises(ValueError, match='outside managed'):
        store.configuration(False, 8080, workspace / 'ops/config/server.toml')


def test_empty_comments_and_accounting_names_are_supported_without_inferring_absence():
    output = '10|gb-deploy-control|RUNNING|compute-a|\n'
    assert parse_jobs(output)[0].comment == ''
    runner = Mock(side_effect=[subprocess.CompletedProcess([], 0, '', ''), subprocess.CompletedProcess([], 0, '', '')])
    assert Slurm(runner).lookup('10', '2026-10-15') is None
    failing = Slurm(Mock(side_effect=SchedulerError('failed')))
    with pytest.raises(SchedulerError): failing.lookup('10', '2026-10-15')
    with pytest.raises(SchedulerError): parse_jobs('broken')
    with pytest.raises(SchedulerError): parse_jobs('1|job|NEW_UNKNOWN_STATE|node|tag')


class FakeApi:
    def __init__(self, count=2):
        self.request_data = {'id': '1', 'lease_token': 'token', 'args': {'groups': [
            {'count': count, 'node_names': [f'w-{i}' for i in range(count)], 'config': {'gpus': 'h100:1', 'partition': 'gpu'},
             'capabilities': {'gpu': 1}, 'max_start_failures': 3}]},
            'workers': [{'node_name': f'w-{i}', 'submission_key': f'key-{i}', 'state': 'pending', 'job_id': None, 'registered': False} for i in range(count)]}
        self.calls = []

    def request(self, path, payload=None, method='POST'):
        self.calls.append((path, payload))
        if path.endswith('claim-external'):
            if all(w['state'] in ('failed','connected','canceled') for w in self.request_data['workers']): return {'request': None}
            return {'request': json.loads(json.dumps(self.request_data))}
        if path.endswith('progress'):
            worker = next(w for w in self.request_data['workers'] if w['node_name'] == payload['node_name'])
            worker['state'] = payload['state']
            if payload['job_id']: worker['job_id'] = payload['job_id']
            return {'ok': True}
        return {'ok': True}


def test_interrupted_worker_submission_is_adopted_then_remaining_workers_launch(workspace):
    manifest = create_manifest(workspace); manifest['control_job_id'] = '10'
    scheduler = FakeSlurm(); scheduler.interrupt_after_accept = True
    api = FakeApi()
    with pytest.raises(KeyboardInterrupt): u.resolve_launch_requests(manifest, 'compute-a', api, scheduler)
    assert api.request_data['workers'][0]['state'] == 'submitting'
    assert api.request_data['workers'][1]['state'] == 'pending'
    u.resolve_launch_requests(manifest, 'compute-a', api, scheduler)
    assert len(scheduler.submissions) == 2
    assert [w['state'] for w in api.request_data['workers']] == ['submitted', 'submitted']
    assert len({w['job_id'] for w in api.request_data['workers']}) == 2


def test_ambiguous_submission_is_never_blindly_retried(workspace):
    manifest = create_manifest(workspace)
    api = FakeApi(1); api.request_data['workers'][0]['state'] = 'submitting'
    scheduler = FakeSlurm()
    u.resolve_launch_requests(manifest, 'compute-a', api, scheduler)
    assert not scheduler.submissions
    assert api.request_data['workers'][0]['state'] == 'submitting'


def test_ended_job_is_failed_but_registered_worker_is_fulfilled(workspace):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm(); api = FakeApi(2)
    for i, worker in enumerate(api.request_data['workers']):
        worker.update(state='submitted', job_id=str(i + 10))
        scheduler.jobs[str(i + 10)] = Job(str(i + 10), job_name(manifest, worker['submission_key']), 'FAILED', 'compute-a', tag(manifest, worker['submission_key']))
    api.request_data['workers'][1]['registered'] = True
    u.resolve_launch_requests(manifest, 'compute-a', api, scheduler)
    assert [w['state'] for w in api.request_data['workers']] == ['failed', 'connected']
    assert not scheduler.submissions


def test_expired_lease_stops_further_mutation():
    api = FakeApi(1)
    with u.Lease(api, api.request_data) as lease:
        lease.lost.set()
        with pytest.raises(RuntimeError, match='lease'): lease.update('w-0', 'submitted', '10')
    assert not any(path.endswith('progress') for path, _ in api.calls)


def test_typed_gpu_uses_server_capabilities_and_enables_nv(workspace):
    request = FakeApi(1).request_data
    spec = u.launch_groups_for_request(request)['w-0']
    assert u.sbatch_args_from_config(spec['config']) == ['--partition=gpu', '--gpus=h100:1']
    command, _ = u.container_command(create_manifest(workspace), ['gammaboard'], gpu=spec['capabilities']['gpu'] > 0)
    assert '--nv' in command
    for invalid in [{'requested_count': 2, 'args': {'node_names': ['old']}}, {'args': {'groups': []}}]:
        with pytest.raises(ValueError): u.launch_groups_for_request(invalid)


def test_scheduler_outage_does_not_terminate_worker(workspace, monkeypatch):
    manifest = create_manifest(workspace); manifest['control_job_id'] = '10'
    recipe = workspace / 'worker.json'
    recipe.write_text(json.dumps({'manifest': str(workspace / 'deployments' / manifest['id'] / 'manifest.json'),
        'database_node': 'compute-a', 'node_name': 'worker', 'max_start_failures': 3, 'capabilities': {}}))
    # Persist the control id and simulate one outage followed by natural worker exit.
    Deployments(workspace, 0).save(manifest)
    worker = Mock(); worker.poll.side_effect = [None, 0, 0, 0]; worker.returncode = 0
    monkeypatch.setattr(u.subprocess, 'Popen', lambda *a, **k: worker)
    monkeypatch.setattr(u.signal, 'signal', lambda *a: None)
    class FastEvent:
        def wait(self, *_): return False
        def set(self): pass
    monkeypatch.setattr(u.threading, 'Event', FastEvent)
    scheduler = FakeSlurm(); scheduler.error = True
    assert u.run_worker(argparse_args(spec=str(recipe)), scheduler) == 0
    worker.terminate.assert_not_called(); worker.kill.assert_not_called()


def argparse_args(**kw):
    from types import SimpleNamespace
    return SimpleNamespace(**kw)


def test_invalid_sync_targets_are_rejected_before_connecting(tmp_path):
    commands = tmp_path / 'commands'; bindir = tmp_path / 'bin'; bindir.mkdir()
    for name in ('ssh', 'rsync'):
        executable = bindir / name
        executable.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "$0" "$@" >> "$SYNC_LOG"\n')
        executable.chmod(0o755)
    env = dict(os.environ, PATH=f'{bindir}:{os.environ["PATH"]}', SYNC_LOG=str(commands))
    script = ROOT / 'sync_ops.sh'
    for invalid in ('../outside', '.', '/absolute', 'user/./workspace'):
        result = subprocess.run(['bash', str(script), 'test-host', invalid], cwd=tmp_path, env=env, capture_output=True)
        assert result.returncode == 2
    assert not commands.exists()



@pytest.mark.parametrize('state', ['PENDING', 'FAILED'])
def test_down_cancels_stranded_workers_and_records_completed_shutdown(workspace, monkeypatch, state):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm(); control(manifest, scheduler, state)
    scheduler.jobs['20'] = Job('20', job_name(manifest, 'worker'), 'PENDING', '', tag(manifest, 'worker'))
    monkeypatch.setattr(u, 'Api', Mock(side_effect=AssertionError('no reachable API')))
    u.down(u.parser().parse_args(['down']), scheduler)
    assert '20' in scheduler.canceled
    assert Deployments(workspace, 0).load()['state'] == 'stopped'
    scheduler.jobs.clear()
    monkeypatch.setattr(u, 'wait_ready', lambda *_: ('compute-a', None))
    u.up(u.parser().parse_args(['up']), scheduler)
    assert Deployments(workspace, 0).load()['id'] != manifest['id']


def test_shutdown_blocks_watchers_from_submitting_more_workers(workspace):
    manifest = create_manifest(workspace)
    manifest['state'] = 'stopping'; Deployments(workspace, 0).save(manifest)
    api = FakeApi(); scheduler = FakeSlurm()
    with pytest.raises(RuntimeError, match='stopping'):
        u.resolve_launch_requests(manifest, 'compute-a', api, scheduler)
    assert not api.calls and not scheduler.submissions


def test_manual_repair_requires_matching_job_or_confirmed_absence(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm(); control(manifest, scheduler)
    api = FakeApi(1); api.request_data['workers'][0]['state'] = 'submitting'
    monkeypatch.setattr(u, 'Api', lambda *_: api)
    scheduler.jobs['20'] = Job('20', 'another-deployment', 'RUNNING', 'compute-b', '')
    with pytest.raises(ValueError, match='does not match'):
        u.resolve_submission(u.parser().parse_args(['resolve-submission', '1', 'w-0', '--job-id', '20']), scheduler)
    scheduler.error = True
    with pytest.raises(SchedulerError):
        u.resolve_submission(u.parser().parse_args(['resolve-submission', '1', 'w-0', '--not-submitted']), scheduler)
    assert api.request_data['workers'][0]['state'] == 'submitting'
    scheduler.error = False
    u.resolve_submission(u.parser().parse_args(['resolve-submission', '1', 'w-0', '--not-submitted']), scheduler)
    assert api.request_data['workers'][0]['state'] == 'pending'
    assert not scheduler.submissions
