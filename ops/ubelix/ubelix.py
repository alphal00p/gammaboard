#!/usr/bin/env python3
"""Deployment-scoped cluster control. Run on a login node with Python 3.11+."""
from __future__ import annotations

import argparse
import base64
from contextlib import suppress
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid

from gbops.deployment import Deployments, digest, tag, write_json, job_name, owns
from gbops.slurm import Slurm, SchedulerError, run

WORKSPACE_ROOT = os.environ.get('GAMMABOARD_WORKSPACE_ROOT', str(Path(__file__).resolve().parent))
SBATCH_OPTIONS = {name: '--' + name.replace('_', '-') for name in
                  ('account', 'partition', 'qos', 'wckey', 'reservation', 'gres', 'gpus',
                   'cpus_per_task', 'mem', 'mem_per_cpu', 'time', 'constraint', 'nodelist', 'exclude')}


def opener():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}))


def shifted_port(base, offset):
    value = base + offset
    if not 0 < value <= 65535: raise ValueError('port out of range')
    return value


def database_url(node, offset):
    return f'postgresql://postgres@{node}:{shifted_port(5400, offset)}/gammaboard_db'


def tunnel_command(node, local_port, offset):
    target = os.environ.get('SSH_TARGET') or f'{os.environ.get("SSH_USER", os.environ.get("USER", ""))}@{os.environ.get("SSH_HOST", "submit03.unibe.ch")}'
    return f'ssh -N -L {local_port}:{node}:{shifted_port(8080, offset)} {target}'


class Api:
    def __init__(self, manifest, node, password=''):
        self.base = f'http://{node}:{8080 + manifest["port_offset"]}/api'
        self.password = password
        self.cookie = None
        health = self.request('/health', method='GET')
        if health.get('deployment_id') != manifest['id'] or health.get('build_id') != manifest['image']['build_info']['build_id']:
            raise RuntimeError('API identity does not match the selected deployment')
        self.login()

    def login(self):
        request = urllib.request.Request(self.base + '/auth/login', data=json.dumps({'password': self.password}).encode(), headers={'Content-Type': 'application/json'})
        with opener().open(request, timeout=10) as response:
            self.cookie = response.headers.get('Set-Cookie', '').split(';')[0]

    def request(self, path, payload=None, method='POST'):
        headers = {'Content-Type': 'application/json'}
        if self.cookie: headers['Cookie'] = self.cookie
        request = urllib.request.Request(self.base + path, data=None if method == 'GET' else json.dumps(payload or {}).encode(), headers=headers, method=method)
        with opener().open(request, timeout=10) as response:
            return json.load(response)


def launch_groups_for_request(request):
    groups = request.get('args', {}).get('groups')
    if not isinstance(groups, list) or not groups:
        raise ValueError('external launches require nonempty args.groups; recreate old requests')
    specs = {}
    for group in groups:
        names = group.get('node_names')
        count = group.get('count')
        if not isinstance(count, int) or isinstance(count, bool) or count < 1 or not isinstance(names, list) or len(names) != count:
            raise ValueError('each group requires its reserved node_names and positive count')
        config, caps = group.get('config'), group.get('capabilities')
        if not isinstance(config, dict) or not isinstance(caps, dict):
            raise ValueError('group requires server-normalized config and capabilities')
        if any(key in config for key in ('gpu', 'cores', 'cpus', 'nr_cores', 'cpus-per-task')):
            raise ValueError('launcher received a noncanonical resource configuration')
        for name in names:
            if not isinstance(name, str) or not name or name in specs: raise ValueError('invalid/duplicate reserved worker name')
            specs[name] = {'node_name': name, 'config': config, 'capabilities': caps,
                           'max_start_failures': group.get('max_start_failures', 3)}
    return specs


def sbatch_args_from_config(config):
    return [f'{option}={config[key]}' for key, option in SBATCH_OPTIONS.items() if key in config]


class Lease:
    def __init__(self, api, request):
        self.api, self.request = api, request
        self.stop, self.lost = threading.Event(), threading.Event()

    def heartbeat(self):
        while not self.stop.wait(20):
            try: self.api.request(f'/node-launch-requests/{self.request["id"]}/lease', {'lease_token': self.request['lease_token']})
            except Exception:
                self.lost.set()
                return

    def __enter__(self):
        self.thread = threading.Thread(target=self.heartbeat, daemon=True)
        self.thread.start()
        return self

    def check(self):
        if self.lost.is_set(): raise RuntimeError('launcher lease could not be renewed; reconciliation required')

    def update(self, name, state, job_id=None, error=None):
        self.check()
        self.api.request(f'/node-launch-requests/{self.request["id"]}/progress',
                         {'lease_token': self.request['lease_token'], 'node_name': name,
                          'state': state, 'job_id': job_id, 'error': error})

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(timeout=12)
        with suppress(Exception):
            self.api.request(f'/node-launch-requests/{self.request["id"]}/lease',
                             {'lease_token': self.request['lease_token'], 'release': True})


def submit_worker(manifest, node, spec, worker, slurm):
    directory = Path(manifest['workspace']) / 'deployments' / manifest['id']
    recipe = directory / 'workers' / f'{worker["submission_key"]}.json'
    write_json(recipe, {'manifest': str(directory / 'manifest.json'), 'database_node': node, **spec})
    env = dict(os.environ, GAMMABOARD_WORKSPACE_ROOT=manifest['workspace'], GAMMABOARD_WORKER_SPEC=str(recipe),
               GAMMABOARD_MANIFEST=str(directory / 'manifest.json'))
    return slurm.submit(directory / 'launcher/worker.sbatch',
                        name=job_name(manifest, worker["submission_key"]), comment=tag(manifest, worker['submission_key']),
                        workspace=manifest['workspace'], env=env, options=sbatch_args_from_config(spec['config']))


def resolve_launch_requests(manifest, node, api, slurm, owner=None):
    deployments = Deployments(manifest['workspace'], manifest['port_offset'])
    with deployments.locked():
        current = deployments.load()
        if current['id'] != manifest['id'] or current['state'] in ('stopping', 'stopped'):
            raise RuntimeError('deployment is stopping or has been replaced')
        return reconcile_launch_requests(manifest, node, api, slurm, owner)


def reconcile_launch_requests(manifest, node, api, slurm, owner=None):
    """One fair pass. A missing ambiguous submission is never automatically repeated."""
    owner = owner or uuid.uuid4().hex
    seen = set()
    while True:
        request = api.request('/node-launch-requests/claim-external', {'launcher_id': owner}).get('request')
        if request is None: return len(seen)
        with Lease(api, request) as lease:
            if request['id'] in seen: return len(seen)
            seen.add(request['id'])
            specs = launch_groups_for_request(request)
            for worker in request['workers']:
                lease.check()
                name, state = worker['node_name'], worker['state']
                if state in ('connected', 'failed', 'canceled'): continue
                if worker.get('registered'):
                    lease.update(name, 'connected', worker.get('job_id'))
                    continue
                if state == 'pending':
                    spec = specs[name]
                    # Validate all local inputs before committing submission intent.
                    sbatch_args_from_config(spec['config'])
                    lease.update(name, 'submitting')
                    job_id = submit_worker(manifest, node, spec, worker, slurm)
                    lease.update(name, 'submitted', job_id)
                    print(f'request={request["id"]} node={name} job={job_id}')
                elif state == 'submitting':
                    matches = slurm.find(tag(manifest, worker['submission_key']), manifest['created_at'][:10])
                    if len(matches) == 1:
                        job = matches[0]
                        lease.update(name, 'submitted', job.id)
                        if job.terminal: lease.update(name, 'failed', job.id, f'Slurm job ended before registration: {job.state}')
                    else:
                        print(f'request={request["id"]} node={name}: submission uncertain ({len(matches)} matching jobs); reconcile before retry', file=sys.stderr)
                elif state == 'submitted':
                    job = slurm.lookup(worker['job_id'], manifest['created_at'][:10])
                    if job and not owns(job, manifest, worker['submission_key']):
                        raise RuntimeError('scheduler job identity does not match the worker reservation')
                    if job and job.terminal:
                        lease.update(name, 'failed', job.id, f'Slurm job ended before registration: {job.state}')


def control_job(manifest, slurm):
    if not manifest.get('control_job_id'): return None
    job = slurm.lookup(manifest['control_job_id'], manifest['created_at'][:10])
    if job and not owns(job, manifest, 'control'): raise RuntimeError('control job belongs to another deployment')
    return job


def require_deployment(args, slurm):
    deployments = Deployments(WORKSPACE_ROOT, args.port_offset)
    manifest = deployments.load()
    if not manifest: raise RuntimeError('no deployment recorded for this workspace/offset')
    return deployments, manifest


def up(args, slurm):
    deployments = Deployments(WORKSPACE_ROOT, args.port_offset)
    image = deployments.selected_image(args.image)
    config = deployments.configuration(args.single_node, args.local_port, args.server_config, args.runtime_config)
    with deployments.locked():
        manifest = deployments.load()
        if manifest and manifest['state'] == 'stopped':
            manifest = None
        if manifest:
            if manifest.get('control_job_id'):
                job = control_job(manifest, slurm)
                if job is None: raise RuntimeError('control status is unknown; refusing to create another deployment')
                if job.terminal:
                    if any(owns(worker, manifest) for worker in slurm.active()):
                        raise RuntimeError('old deployment still has active jobs; run down before replacing it')
                    manifest = None
            elif manifest['control_submission'] == 'submitting':
                matches = slurm.find(tag(manifest, 'control'), manifest['created_at'][:10])
                if len(matches) != 1: raise RuntimeError('control submission uncertain; use resolve-control after checking Slurm')
                manifest['control_job_id'] = matches[0].id
                manifest['control_submission'] = 'submitted'
                deployments.save(manifest)
                if matches[0].terminal:
                    if any(owns(worker, manifest) for worker in slurm.active()):
                        raise RuntimeError('old deployment still has active jobs; run down before replacing it')
                    manifest = None
        if manifest and manifest['state'] == 'stopping':
            raise RuntimeError('deployment is stopping; wait for its jobs to finish')
        if manifest is not None and (manifest['single_node'] != args.single_node or manifest['config_hash'] != config['hash'] or manifest['image']['sha256'] != image['sha256']):
            raise RuntimeError('active deployment has different image/configuration; stop it before starting the replacement')
        if manifest and args.resume_workers and not manifest['resume_workers']:
            raise RuntimeError('--resume-workers is a startup setting; stop the active deployment before enabling it')
        if manifest is None: manifest = deployments.create(image, config, args.single_node, args.resume_workers)
        if manifest['control_submission'] == 'prepared':
            manifest['control_submission'] = 'submitting'
            deployments.save(manifest)
            for name in ('control', 'workers', 'build'):
                (deployments.workspace / 'logs/slurm' / name).mkdir(parents=True, exist_ok=True)
            env = dict(os.environ, GAMMABOARD_WORKSPACE_ROOT=str(deployments.workspace),
                       GAMMABOARD_MANIFEST=str(deployments.root / manifest['id'] / 'manifest.json'))
            manifest['control_job_id'] = slurm.submit(deployments.root / manifest['id'] / 'launcher/control.sbatch',
                name=job_name(manifest, "control"), comment=tag(manifest, 'control'),
                workspace=deployments.workspace, env=env, options=[f'--time={args.time}'])
            manifest['control_submission'] = 'submitted'
            deployments.save(manifest)
    print(f'deployment={manifest["id"]} control_job={manifest["control_job_id"]}')
    node, api = wait_ready(manifest, slurm, args.startup_timeout, args.admin_password)
    with deployments.locked():
        current = deployments.load()
        if current['id'] != manifest['id'] or current['state'] in ('stopping', 'stopped'):
            raise RuntimeError('deployment stopped or was replaced during startup')
        current['state'] = 'running'
        deployments.save(current)
    tunnel = tunnel_command(node, args.local_port, args.port_offset)
    print(f'tunnel={tunnel}')
    if args.copy and sys.stdout.isatty():
        sys.stdout.write('\033]52;c;' + base64.b64encode(tunnel.encode()).decode() + '\a')
    if args.resume_workers and not args.single_node:
        resolve_launch_requests(manifest, node, api, slurm)
    if args.watch:
        watch(manifest, node, api, slurm, args.poll_seconds)


def wait_ready(manifest, slurm, timeout, password):
    deadline, last = time.monotonic() + timeout, 'not started'
    while time.monotonic() < deadline:
        try:
            job = control_job(manifest, slurm)
            if job and job.terminal: raise RuntimeError(f'control job ended: {job.state}')
            if job and job.node:
                try: return job.node, Api(manifest, job.node, password)
                except (OSError, urllib.error.URLError) as error: last = str(error)
        except SchedulerError as error: last = str(error)
        time.sleep(2)
    raise RuntimeError(f'deployment not ready within timeout ({last}); it remains recorded; down can cancel a pending job')


def down(args, slurm):
    deployments, manifest = require_deployment(args, slurm)
    with deployments.locked():
        manifest = deployments.load()
        if manifest['state'] == 'stopped': return
        if not manifest.get('control_job_id'):
            if manifest['control_submission'] == 'prepared':
                manifest['state'] = 'stopped'
                deployments.save(manifest)
                return
            matches = slurm.find(tag(manifest, 'control'), manifest['created_at'][:10])
            if len(matches) != 1: raise RuntimeError('control submission uncertain; use resolve-control')
            manifest['control_job_id'] = matches[0].id
            deployments.save(manifest)
        job = control_job(manifest, slurm)
        if job is None: raise RuntimeError('control status unknown; no jobs canceled')
        manifest['state'] = 'stopping'
        deployments.save(manifest)
        if job.terminal or job.state == 'PENDING':
            slurm.cancel([w.id for w in slurm.active() if owns(w, manifest) and w.id != job.id])
            if not job.terminal: slurm.cancel([job.id])
        else:
            if not job.node: raise RuntimeError('control has no reachable node yet; retry once its state is known')
            api = Api(manifest, job.node, args.admin_password)
            api.request('/admin/workers/suspend')
            deadline = time.monotonic() + args.worker_timeout
            while True:
                workers = [w for w in slurm.active() if owns(w, manifest) and w.id != job.id]
                if not workers or time.monotonic() >= deadline: break
                time.sleep(2)
            slurm.cancel([w.id for w in workers])
            time.sleep(args.control_grace_seconds)
            slurm.cancel([job.id])
        if not any(owns(w, manifest) for w in slurm.active()):
            final = control_job(manifest, slurm)
            if final and final.terminal: manifest['state'] = 'stopped'
        deployments.save(manifest)
        print(f'shutdown requested for deployment={manifest["id"]}')


def watch(manifest, node, api, slurm, interval, once=False):
    delay = interval
    while True:
        try:
            job = control_job(manifest, slurm)
            if job and job.terminal: return
            if job is None: raise SchedulerError('control status unknown')
            if not manifest['single_node']: resolve_launch_requests(manifest, node, api, slurm)
            if once: return
            delay = interval
        except urllib.error.HTTPError as error:
            if error.code == 401: api.login()
            else: print(f'launcher retry: {error}', file=sys.stderr)
            if once: raise
            delay = min(60, max(interval, delay * 2))
        except (OSError, SchedulerError, RuntimeError) as error:
            if once: raise
            print(f'launcher retry: {error}', file=sys.stderr)
            delay = min(60, max(interval, delay * 2))
        time.sleep(delay)


def container_command(manifest, command, gpu=False):
    image = manifest['image']
    for name in ('server', 'runtime'):
        if digest(manifest[name + '_config']) != manifest[name + '_digest']: raise RuntimeError('deployment configuration snapshot was modified')
    if digest(image['path']) != image['sha256']: raise RuntimeError('pinned image was modified')
    env = dict(os.environ)
    env.update({'APPTAINERENV_GAMMABOARD_DEPLOYMENT_ID': manifest['id'],
                'APPTAINERENV_TZ': 'Etc/UTC', 'APPTAINERENV_LANG': 'C.UTF-8', 'APPTAINERENV_LC_ALL': 'C.UTF-8'})
    args = ['apptainer', 'exec', '--cleanenv', '--bind', manifest['workspace']]
    if gpu: args += ['--nv']
    return [*args, image['path'], *command], env


def read_manifest(path):
    manifest = json.loads(Path(path).read_text())
    if manifest.get('version') != 1: raise ValueError('unsupported deployment manifest')
    return manifest


def run_control(args):
    manifest = read_manifest(args.manifest)
    command = ['gammaboard', '--runtime-config', manifest['runtime_config'], '--resource-root', manifest['workspace'] + '/resources', '--port-offset', str(manifest['port_offset'])]
    if not manifest['single_node']: command += ['--postgres-trusted-network']
    command += ['deploy', '--server-config', manifest['server_config']]
    if manifest['resume_workers']: command += ['--resume-workers']
    for origin in manifest['origins']: command += ['--allowed-origin', origin]
    command, env = container_command(manifest, command)
    os.execvpe(command[0], command, env)


def run_worker(args, slurm):
    spec = json.loads(Path(args.spec).read_text())
    manifest = read_manifest(spec['manifest'])
    caps = spec['capabilities']
    command = ['gammaboard', '--database-url', database_url(spec['database_node'], manifest['port_offset']),
               '--resource-root', manifest['workspace'] + '/resources', 'node', 'run', '--name', spec['node_name'],
               '--max-start-failures', str(spec['max_start_failures'])]
    for name, count in sorted(caps.items()): command += ['--capability', f'{name}={count}']
    command, env = container_command(manifest, command, gpu=caps.get('gpu', 0) > 0)
    stop = threading.Event()
    for sig in (signal.SIGTERM, signal.SIGINT): signal.signal(sig, lambda *_: stop.set())
    worker = subprocess.Popen(command, env=env)
    try:
        while worker.poll() is None and not stop.wait(5):
            try:
                job = control_job(manifest, slurm)
                if job and job.terminal: stop.set()
            except SchedulerError as error:
                print(f'scheduler status unavailable; keeping worker alive: {error}', file=sys.stderr)
    finally:
        if worker.poll() is None:
            worker.terminate()
            try: worker.wait(timeout=55)
            except subprocess.TimeoutExpired:
                worker.kill(); worker.wait()
    return worker.returncode


def resolve_control(args, slurm):
    deployments, manifest = require_deployment(args, slurm)
    with deployments.locked():
        manifest = deployments.load()
        if manifest.get('control_job_id'): raise ValueError('control already has a recorded job')
        matches = slurm.find(tag(manifest, 'control'), manifest['created_at'][:10])
        if args.job_id:
            job = slurm.lookup(args.job_id, manifest['created_at'][:10])
            if not job or not owns(job, manifest, 'control'): raise ValueError('job identity does not match this deployment')
            manifest['control_job_id'], manifest['control_submission'] = job.id, 'submitted'
        else:
            if matches: raise ValueError('matching Slurm jobs exist; cannot confirm absence')
            manifest['control_submission'] = 'prepared'
        deployments.save(manifest)


def resolve_submission(args, slurm):
    deployments, _ = require_deployment(args, slurm)
    with deployments.locked():
        manifest = deployments.load()
        if manifest['state'] in ('stopping', 'stopped'):
            raise RuntimeError('deployment is stopping')
        return repair_submission(args, slurm, manifest)


def repair_submission(args, slurm, manifest):
    job = control_job(manifest, slurm)
    if not job or job.terminal or not job.node: raise RuntimeError('selected deployment is not running')
    api = Api(manifest, job.node, args.admin_password)
    request = api.request('/node-launch-requests/claim-external',
                          {'launcher_id': uuid.uuid4().hex, 'request_id': args.request_id}).get('request')
    if not request: raise RuntimeError('request is finished or another launcher holds its lease; stop the watcher and wait for lease expiry')
    with Lease(api, request) as lease:
        worker = next((w for w in request['workers'] if w['node_name'] == args.node), None)
        if not worker or worker['state'] != 'submitting' or worker.get('registered'):
            raise ValueError('only an unregistered ambiguous submission can be resolved')
        expected = tag(manifest, worker['submission_key'])
        if args.job_id:
            found = slurm.lookup(args.job_id, manifest['created_at'][:10])
            if not found or not owns(found, manifest, worker['submission_key']):
                raise ValueError('job does not match this worker submission')
            lease.update(args.node, 'submitted', found.id)
            if found.terminal: lease.update(args.node, 'failed', found.id, 'resolved job already ended before registration')
        else:
            if slurm.find(expected, manifest['created_at'][:10]): raise ValueError('matching Slurm jobs exist; refusing retry')
            lease.update(args.node, 'pending', error='operator confirmed no submission')
    print('submission resolved; the watcher can continue')


def main(argv=None):
    args = parser().parse_args(argv)
    slurm = Slurm()
    try:
        if args.command == 'run-control': run_control(args); return
        if args.command == 'run-worker': return run_worker(args, slurm)
        if args.command == 'up': up(args, slurm)
        elif args.command == 'down': down(args, slurm)
        elif args.command == 'resolve-control': resolve_control(args, slurm)
        elif args.command == 'resolve-submission': resolve_submission(args, slurm)
        elif args.command == 'build':
            workspace = Path(WORKSPACE_ROOT).resolve()
            (workspace / 'logs/slurm/build').mkdir(parents=True, exist_ok=True)
            env = dict(os.environ, GAMMABOARD_WORKSPACE_ROOT=str(workspace))
            if args.build_kind == 'apptainer':
                script = workspace / 'ops/build/build.sbatch'
                options = [str(script), args.output, args.def_file]
            else:
                env['GAMMABOARD_REVISION' if args.build_kind == 'gammaboard' else 'GAMMALOOP_REVISION'] = args.revision
                options = [str(workspace / 'ops/build' / f'{args.build_kind}.sbatch')]
            print(run(['sbatch', '--parsable', '--chdir', str(workspace), *options], env=env).stdout.strip())
        else:
            _, manifest = require_deployment(args, slurm)
            job = control_job(manifest, slurm)
            if args.command == 'status':
                print(json.dumps({'deployment': manifest['id'], 'control': None if job is None else job.__dict__,
                    'state': manifest['state'], 'image': manifest['image']['path'],
                    'image_sha256': manifest['image']['sha256'], 'build_id': manifest['image']['build_info']['build_id'],
                    'workers': [j.__dict__ for j in slurm.active() if owns(j, manifest) and j.id != manifest['control_job_id']]}, indent=2))
                return
            if not job or job.terminal or not job.node: raise RuntimeError('selected deployment is not running')
            api = Api(manifest, job.node, args.admin_password)
            if args.command == 'submit-workers':
                from gbops.deployment import toml_dump
                config = dict(args.capabilities)
                toml = f'[[groups]]\ncount={args.count}\nname_prefix={json.dumps(args.prefix)}\nmax_start_failures={args.max_start_failures}\n[groups.config]\n' + toml_dump(config)
                print(json.dumps(api.request('/node-launch-requests', {'toml': toml})))
            watch(manifest, job.node, api, slurm, args.poll_seconds, once=args.command == 'submit-workers' or args.once)
    except KeyboardInterrupt:
        print('launcher stopped; recorded submissions remain recoverable')
    except (OSError, ValueError, RuntimeError, urllib.error.URLError) as error:
        raise SystemExit(str(error)) from error


def positive(value):
    n = int(value)
    if n <= 0: raise argparse.ArgumentTypeError('must be positive')
    return n


def port(value):
    n = positive(value)
    if n > 65535: raise argparse.ArgumentTypeError("port out of range")
    return n


def offset(value):
    n = int(value)
    if not 0 <= n <= 65535 - 8080: raise argparse.ArgumentTypeError('port offset out of range')
    return n


def parse_capability(value):
    key, separator, raw = value.partition('=')
    if not separator or not key: raise argparse.ArgumentTypeError('capability must use KEY=VALUE')
    return key, positive(raw)


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    commands = p.add_subparsers(dest='command', required=True)
    for name in ('up', 'down', 'status', 'submit-workers', 'watch-requests', 'resolve-control', 'resolve-submission'):
        cmd = commands.add_parser(name)
        cmd.add_argument('--port-offset', type=offset, default=0)
        cmd.add_argument('--admin-password', default=os.environ.get('GAMMABOARD_ADMIN_PASSWORD', ''))
        if name == 'up':
            cmd.add_argument('--time', default='00:20:00')
            cmd.add_argument('--single-node', action='store_true')
            cmd.add_argument('--resume-workers', action='store_true')
            cmd.add_argument('--local-port', type=port, default=8080)
            cmd.add_argument('--startup-timeout', type=positive, default=180)
            cmd.add_argument('--poll-seconds', type=positive, default=5)
            cmd.add_argument('--watch', action='store_true')
            cmd.add_argument('--copy', action='store_true')
            cmd.add_argument('--image', help='immutable image metadata JSON; defaults to images/gammaboard/current.json')
            cmd.add_argument('--server-config', help='private server override TOML')
            cmd.add_argument('--runtime-config', help='private runtime override TOML')
        elif name == 'down':
            cmd.add_argument('--worker-timeout', type=int, default=60)
            cmd.add_argument('--control-grace-seconds', type=int, default=5)
        elif name in ('submit-workers', 'watch-requests'):
            cmd.add_argument('--poll-seconds', type=positive, default=5)
            cmd.add_argument('--once', action='store_true')
            if name == 'submit-workers':
                cmd.add_argument('--count', type=positive, required=True)
                cmd.add_argument('--prefix', default='w')
                cmd.add_argument('--max-start-failures', type=int, default=3)
                cmd.add_argument('--capability', dest='capabilities', action='append', default=[], type=parse_capability)
        elif name in ('resolve-control', 'resolve-submission'):
            if name == 'resolve-submission':
                cmd.add_argument('request_id', type=positive)
                cmd.add_argument('node')
            choice = cmd.add_mutually_exclusive_group(required=True)
            choice.add_argument('--job-id')
            choice.add_argument('--not-submitted', action='store_true', help='confirm, after checking Slurm, that no job was submitted')
    build = commands.add_parser('build').add_subparsers(dest='build_kind', required=True)
    for name in ('gammaboard', 'gammaloop', 'apptainer'):
        cmd = build.add_parser(name)
        if name == 'apptainer': cmd.add_argument('output'); cmd.add_argument('def_file')
        else: cmd.add_argument('--revision', default='HEAD')
    commands.add_parser('run-control').add_argument('--manifest', required=True)
    commands.add_parser('run-worker').add_argument('--spec', required=True)
    return p


if __name__ == '__main__':
    sys.exit(main())
