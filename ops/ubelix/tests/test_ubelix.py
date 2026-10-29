import hashlib
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
from gbops.deployment import Deployments, tag, job_name, write_json
from gbops.slurm import Job, Slurm, SchedulerError, parse_jobs
from gbops import runtime


@pytest.fixture
def workspace(tmp_path, monkeypatch):
    monkeypatch.setattr(u, "WORKSPACE_ROOT", str(tmp_path))
    (tmp_path / "ops/config").mkdir(parents=True)
    for name in ("server.toml", "server-single-node.toml", "cluster.toml"):
        shutil.copy2(ROOT / "config" / name, tmp_path / "ops/config" / name)
    shutil.copy2(ROOT / "ubelix.py", tmp_path / "ubelix.py")
    shutil.copytree(
        ROOT / "gbops", tmp_path / "gbops", ignore=shutil.ignore_patterns("__pycache__")
    )
    shutil.copytree(ROOT / "slurm", tmp_path / "ops/slurm")
    image = tmp_path / "images/gammaboard/test.sif"
    image.parent.mkdir(parents=True)
    image.write_bytes(b"first immutable image")
    write_json(
        image.parent / "current.json",
        {
            "path": str(image),
            "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "build_info": {"build_id": "test-build", "model_version": 7},
            "validated": True,
        },
    )
    return tmp_path


class FakeSlurm(Slurm):
    def __init__(self):
        super().__init__()
        self.jobs = {}
        self.submissions = []
        self.canceled = []
        self.interrupt_after_accept = False
        self.error = False

    def active(self):
        if self.error:
            raise SchedulerError("scheduler unavailable")
        return [j for j in self.jobs.values() if not j.terminal]

    def lookup(self, identifier, since):
        if self.error:
            raise SchedulerError("scheduler unavailable")
        return self.jobs.get(identifier)

    def find(self, comment, since):
        if self.error:
            raise SchedulerError("accounting unavailable")
        return [j for j in self.jobs.values() if j.comment == comment]

    def submit(self, script, **kw):
        identifier = str(100 + len(self.submissions))
        self.submissions.append((script, kw))
        self.jobs[identifier] = Job(identifier, kw["name"], "PENDING", "", kw["comment"])
        if self.interrupt_after_accept:
            self.interrupt_after_accept = False
            raise KeyboardInterrupt()
        return identifier

    def cancel(self, ids):
        self.canceled += ids
        for identifier in ids:
            j = self.jobs[identifier]
            self.jobs[identifier] = Job(j.id, j.name, "CANCELLED", j.node, j.comment)


def create_manifest(workspace, offset=0):
    store = Deployments(workspace, offset)
    return store.create(store.selected_image(), store.configuration(False, 8080), False, False)


def control(manifest, scheduler, state="RUNNING"):
    j = Job(
        "10",
        job_name(manifest, "control"),
        state,
        "compute-a" if state != "PENDING" else "",
        tag(manifest, "control"),
    )
    scheduler.jobs[j.id] = j
    manifest["control_job_id"], manifest["control_submission"] = j.id, "submitted"
    Deployments(manifest["workspace"], manifest["port_offset"]).save(manifest)
    return j


def test_workspace_and_offset_are_separate_and_pending_down_never_calls_api(workspace, monkeypatch):
    scheduler = FakeSlurm()
    monkeypatch.setattr(u, "wait_ready", lambda *_: ("compute-a", None))
    u.up(u.parser().parse_args(["up"]), scheduler)
    first = Deployments(workspace, 0).load()
    u.up(u.parser().parse_args(["up", "--port-offset", "10"]), scheduler)
    second = Deployments(workspace, 10).load()
    assert first["id"] != second["id"]
    assert len(scheduler.submissions) == 2
    monkeypatch.setattr(u, "Api", Mock(side_effect=AssertionError("pending jobs have no API")))
    u.down(u.parser().parse_args(["down", "--port-offset", "10"]), scheduler)
    assert scheduler.canceled == [second["control_job_id"]]
    assert not scheduler.jobs[first["control_job_id"]].terminal
    other = workspace / "other"
    other.mkdir()
    assert Deployments(other, 0).slot != Deployments(workspace, 0).slot


def test_control_submission_is_recovered_without_resubmitting(workspace, monkeypatch):
    scheduler = FakeSlurm()
    scheduler.interrupt_after_accept = True
    monkeypatch.setattr(u, "wait_ready", lambda *_: ("compute-a", None))
    with pytest.raises(KeyboardInterrupt):
        u.up(u.parser().parse_args(["up"]), scheduler)
    assert Deployments(workspace, 0).load()["control_submission"] == "submitting"
    u.up(u.parser().parse_args(["up"]), scheduler)
    assert len(scheduler.submissions) == 1
    assert Deployments(workspace, 0).load()["control_job_id"] == "100"


def test_unknown_control_or_scheduler_error_never_creates_replacement(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    manifest["control_submission"] = "submitting"
    Deployments(workspace, 0).save(manifest)
    scheduler = FakeSlurm()
    with pytest.raises(RuntimeError, match="uncertain"):
        u.up(u.parser().parse_args(["up"]), scheduler)
    scheduler.error = True
    with pytest.raises(SchedulerError):
        u.up(u.parser().parse_args(["up"]), scheduler)
    assert not scheduler.submissions


def test_running_down_only_cancels_its_own_jobs(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs["20"] = Job(
        "20", job_name(manifest, "worker"), "RUNNING", "compute-b", tag(manifest, "worker")
    )
    scheduler.jobs["30"] = Job("30", "gb-other-control", "RUNNING", "compute-c", "gb:other:control")
    api = Mock()
    monkeypatch.setattr(u, "Api", lambda *_: api)
    u.down(
        u.parser().parse_args(["down", "--worker-timeout", "0", "--cancel-timeout", "1"]), scheduler
    )
    api.request.assert_called_once_with("/admin/workers/suspend")
    assert scheduler.canceled == ["20", "10"]
    assert not scheduler.jobs["30"].terminal


def test_active_deployment_pins_image_and_rejects_changed_current_pointer(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    launcher = workspace / "deployments" / manifest["id"] / "launcher/ubelix.py"
    original = launcher.read_text()
    (workspace / "ubelix.py").write_text("# changed by sync\n")
    assert launcher.read_text() == original
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    image = workspace / "images/gammaboard/second.sif"
    image.write_bytes(b"second version")
    write_json(
        image.parent / "current.json",
        {
            "path": str(image),
            "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "build_info": {"build_id": "other-build", "model_version": 7},
            "validated": True,
        },
    )
    with pytest.raises(RuntimeError, match="different image"):
        u.up(u.parser().parse_args(["up"]), scheduler)
    command, env = u.container_command(manifest, ["gammaboard", "build-info"])
    assert manifest["image"]["path"] in command
    assert str(image) not in command
    assert env["APPTAINERENV_GAMMABOARD_DEPLOYMENT_ID"] == manifest["id"]


def test_custom_tunnel_origin_and_private_configuration_are_snapshotted(workspace):
    (workspace / "config").mkdir()
    private = workspace / "config/server.toml"
    private.write_text(
        'name="private"\n[auth]\nadmin_password_hash="hash"\nsession_secret="secret"\n'
    )
    store = Deployments(workspace, 10)
    config = store.configuration(False, 9000)
    manifest = store.create(
        store.selected_image(), config, False, False, password="private-password"
    )
    assert manifest["origins"] == ["http://localhost:9000", "http://127.0.0.1:9000"]
    assert "private" in Path(manifest["server_config"]).read_text()
    assert "9000:compute-a:8090" in u.tunnel_command("compute-a", 9000, 10)
    credentials = runtime.directory(manifest) / "launcher-auth.json"
    assert credentials.stat().st_mode & 0o777 == 0o600
    assert "private-password" not in json.dumps(manifest)
    private.write_text('name="changed"')
    assert "changed" not in Path(manifest["server_config"]).read_text()
    Path(manifest["server_config"]).write_text('name="tampered"')
    with pytest.raises(RuntimeError, match="snapshot"):
        u.container_command(manifest, [])
    with pytest.raises(ValueError, match="outside managed"):
        store.configuration(False, 8080, workspace / "ops/config/server.toml")


def test_empty_comments_and_accounting_names_are_supported_without_inferring_absence():
    output = "10|gb-deploy-control|RUNNING|compute-a|\n"
    assert parse_jobs(output)[0].comment == ""
    runner = Mock(
        side_effect=[
            subprocess.CompletedProcess([], 0, "", ""),
            subprocess.CompletedProcess([], 0, "", ""),
        ]
    )
    assert Slurm(runner).lookup("10", "2026-10-15") is None
    failing = Slurm(Mock(side_effect=SchedulerError("failed")))
    with pytest.raises(SchedulerError):
        failing.lookup("10", "2026-10-15")
    with pytest.raises(SchedulerError):
        parse_jobs("broken")
    with pytest.raises(SchedulerError):
        parse_jobs("1|job|NEW_UNKNOWN_STATE|node|tag")


class FakeApi:
    def __init__(self, count=2):
        self.request_data = {
            "id": "1",
            "lease_token": "token",
            "args": {
                "groups": [
                    {
                        "count": count,
                        "node_names": [f"w-{i}" for i in range(count)],
                        "config": {"gpus": "h100:1", "partition": "gpu"},
                        "capabilities": {"gpu": 1},
                        "max_start_failures": 3,
                    }
                ]
            },
            "workers": [
                {
                    "node_name": f"w-{i}",
                    "submission_key": f"key-{i}",
                    "state": "pending",
                    "job_id": None,
                    "registered": False,
                }
                for i in range(count)
            ],
        }
        self.calls = []

    def request(self, path, payload=None, method="POST"):
        self.calls.append((path, payload))
        if path.endswith("claim-external"):
            if all(
                w["state"] in ("failed", "connected", "canceled")
                for w in self.request_data["workers"]
            ):
                return {"request": None}
            return {"request": json.loads(json.dumps(self.request_data))}
        if path.endswith("progress"):
            worker = next(
                w for w in self.request_data["workers"] if w["node_name"] == payload["node_name"]
            )
            worker["state"] = payload["state"]
            if payload["job_id"]:
                worker["job_id"] = payload["job_id"]
            return {"ok": True}
        return {"ok": True}


def test_interrupted_worker_submission_is_adopted_then_remaining_workers_launch(workspace):
    manifest = create_manifest(workspace)
    manifest["control_job_id"] = "10"
    scheduler = FakeSlurm()
    scheduler.interrupt_after_accept = True
    api = FakeApi()
    with pytest.raises(KeyboardInterrupt):
        u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert api.request_data["workers"][0]["state"] == "submitting"
    assert api.request_data["workers"][1]["state"] == "pending"
    u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert len(scheduler.submissions) == 2
    assert [w["state"] for w in api.request_data["workers"]] == ["submitted", "submitted"]
    assert len({w["job_id"] for w in api.request_data["workers"]}) == 2


def test_ambiguous_submission_is_never_blindly_retried(workspace):
    manifest = create_manifest(workspace)
    api = FakeApi(1)
    api.request_data["workers"][0]["state"] = "submitting"
    scheduler = FakeSlurm()
    u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert not scheduler.submissions
    assert api.request_data["workers"][0]["state"] == "submitting"


def test_ended_job_is_failed_but_registered_worker_is_fulfilled(workspace):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    api = FakeApi(2)
    for i, worker in enumerate(api.request_data["workers"]):
        worker.update(state="submitted", job_id=str(i + 10))
        scheduler.jobs[str(i + 10)] = Job(
            str(i + 10),
            job_name(manifest, worker["submission_key"]),
            "FAILED",
            "compute-a",
            tag(manifest, worker["submission_key"]),
        )
    api.request_data["workers"][1]["registered"] = True
    u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert [w["state"] for w in api.request_data["workers"]] == ["failed", "connected"]
    assert not scheduler.submissions


def test_expired_lease_stops_further_mutation():
    api = FakeApi(1)
    with u.Lease(api, api.request_data) as lease:
        lease.lost.set()
        with pytest.raises(RuntimeError, match="lease"):
            lease.update("w-0", "submitted", "10")
    assert not any(path.endswith("progress") for path, _ in api.calls)


def test_typed_gpu_uses_server_capabilities_and_enables_nv(workspace):
    request = FakeApi(1).request_data
    spec = u.launch_groups_for_request(request)["w-0"]
    assert u.sbatch_args_from_config(spec["config"]) == ["--partition=gpu", "--gpus=h100:1"]
    command, _ = u.container_command(
        create_manifest(workspace), ["gammaboard"], gpu=spec["capabilities"]["gpu"] > 0
    )
    assert "--nv" in command
    for invalid in [
        {"requested_count": 2, "args": {"node_names": ["old"]}},
        {"args": {"groups": []}},
    ]:
        with pytest.raises(ValueError):
            u.launch_groups_for_request(invalid)


def test_scheduler_outage_does_not_terminate_worker(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    manifest["control_job_id"] = "10"
    recipe = workspace / "worker.json"
    recipe.write_text(
        json.dumps(
            {
                "manifest": str(workspace / "deployments" / manifest["id"] / "manifest.json"),
                "database_node": "compute-a",
                "node_name": "worker",
                "max_start_failures": 3,
                "capabilities": {},
            }
        )
    )
    # Persist the control id and simulate one outage followed by natural worker exit.
    Deployments(workspace, 0).save(manifest)
    worker = Mock()
    worker.poll.side_effect = [None, 0, 0, 0]
    worker.returncode = 0
    monkeypatch.setattr(u.subprocess, "Popen", lambda *a, **k: worker)
    monkeypatch.setattr(runtime.signal, "signal", lambda *a: None)

    class FastEvent:
        def wait(self, *_):
            return False

        def set(self):
            pass

    monkeypatch.setattr(u.threading, "Event", FastEvent)
    scheduler = FakeSlurm()
    scheduler.error = True
    assert u.run_worker(argparse_args(spec=str(recipe)), scheduler) == 0
    worker.terminate.assert_not_called()
    worker.kill.assert_not_called()


def argparse_args(**kw):
    from types import SimpleNamespace

    return SimpleNamespace(**kw)


def test_invalid_sync_targets_are_rejected_before_connecting(tmp_path):
    commands = tmp_path / "commands"
    bindir = tmp_path / "bin"
    bindir.mkdir()
    for name in ("ssh", "rsync"):
        executable = bindir / name
        executable.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "$0" "$@" >> "$SYNC_LOG"\n')
        executable.chmod(0o755)
    env = dict(os.environ, PATH=f"{bindir}:{os.environ['PATH']}", SYNC_LOG=str(commands))
    script = ROOT / "sync_ops.sh"
    for invalid in ("../outside", ".", "/absolute", "user/./workspace"):
        result = subprocess.run(
            ["bash", str(script), "test-host", invalid], cwd=tmp_path, env=env, capture_output=True
        )
        assert result.returncode == 2
    assert not commands.exists()


@pytest.mark.parametrize("state", ["PENDING", "FAILED"])
def test_down_cancels_stranded_workers_and_records_completed_shutdown(
    workspace, monkeypatch, state
):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler, state)
    scheduler.jobs["20"] = Job(
        "20", job_name(manifest, "worker"), "PENDING", "", tag(manifest, "worker")
    )
    monkeypatch.setattr(u, "Api", Mock(side_effect=AssertionError("no reachable API")))
    u.down(u.parser().parse_args(["down"]), scheduler)
    assert "20" in scheduler.canceled
    assert Deployments(workspace, 0).load()["state"] == "stopped"
    scheduler.jobs.clear()
    monkeypatch.setattr(u, "wait_ready", lambda *_: ("compute-a", None))
    u.up(u.parser().parse_args(["up"]), scheduler)
    assert Deployments(workspace, 0).load()["id"] != manifest["id"]


def test_shutdown_blocks_watchers_from_submitting_more_workers(workspace):
    manifest = create_manifest(workspace)
    manifest["state"] = "stopping"
    Deployments(workspace, 0).save(manifest)
    api = FakeApi()
    scheduler = FakeSlurm()
    with pytest.raises(RuntimeError, match="stopping"):
        u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert not api.calls and not scheduler.submissions


def test_manual_repair_requires_matching_job_or_confirmed_absence(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    api = FakeApi(1)
    api.request_data["workers"][0]["state"] = "submitting"
    monkeypatch.setattr(u, "Api", lambda *_: api)
    scheduler.jobs["20"] = Job("20", "another-deployment", "RUNNING", "compute-b", "")
    with pytest.raises(ValueError, match="does not match"):
        u.resolve_submission(
            u.parser().parse_args(["resolve-submission", "1", "w-0", "--job-id", "20"]), scheduler
        )
    scheduler.error = True
    with pytest.raises(SchedulerError):
        u.resolve_submission(
            u.parser().parse_args(["resolve-submission", "1", "w-0", "--not-submitted"]), scheduler
        )
    assert api.request_data["workers"][0]["state"] == "submitting"
    scheduler.error = False
    u.resolve_submission(
        u.parser().parse_args(["resolve-submission", "1", "w-0", "--not-submitted"]), scheduler
    )
    assert api.request_data["workers"][0]["state"] == "pending"
    assert not scheduler.submissions


def test_force_down_works_without_api_and_waits_for_workers_before_control(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs["20"] = Job(
        "20", job_name(manifest, "worker"), "RUNNING", "compute-b", tag(manifest, "worker")
    )
    scheduler.jobs["30"] = Job("30", "unrelated-job", "RUNNING", "other", "")
    monkeypatch.setattr(u, "Api", Mock(side_effect=OSError("API unreachable")))
    with pytest.raises(RuntimeError, match="down --force"):
        u.down(u.parser().parse_args(["down", "--worker-timeout", "0"]), scheduler)
    assert scheduler.canceled == []
    assert Deployments(workspace, 0).load()["state"] == "stopping"
    cancel = scheduler.cancel

    def ordered_cancel(ids):
        if "10" in ids:
            assert scheduler.jobs["20"].terminal
        cancel(ids)

    scheduler.cancel = ordered_cancel
    u.down(u.parser().parse_args(["down", "--force"]), scheduler)
    assert scheduler.canceled == ["20", "10"]
    assert not scheduler.jobs["30"].terminal
    assert Deployments(workspace, 0).load()["state"] == "stopped"


def test_force_down_refuses_unknown_or_foreign_control(workspace):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs.clear()
    with pytest.raises(RuntimeError, match="unknown"):
        u.down(u.parser().parse_args(["down", "--force"]), scheduler)
    scheduler.jobs["10"] = Job("10", "unrelated", "RUNNING", "other", "")
    with pytest.raises(RuntimeError, match="another deployment"):
        u.down(u.parser().parse_args(["down", "--force"]), scheduler)
    assert not scheduler.canceled


def test_worker_uses_pinned_runtime_and_does_not_double_apply_offset(workspace):
    manifest = create_manifest(workspace, 10)
    command = runtime.runtime_command(manifest, "compute-a")
    assert command[command.index("--runtime-config") + 1] == manifest["runtime_config"]
    assert (
        command[command.index("--database-url") + 1]
        == "postgresql://postgres@compute-a:5410/gammaboard_db"
    )
    assert "--port-offset" not in command


def test_database_reset_requires_shutdown_and_is_scoped_to_offset(workspace):
    from gbops.maintenance import reset_database

    manifest = create_manifest(workspace, 10)
    paths = manifest["database_paths"]
    data = Path(paths["data_dir"])
    data.mkdir(parents=True)
    (data / "PG_VERSION").write_text("15")
    (workspace / "resources/db/postgres").mkdir()
    (workspace / "resources/db/postgres/keep").write_text("other offset")
    physics = workspace / "resources/states/private"
    physics.mkdir(parents=True)
    (physics / "keep").write_text("physics")
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    deployments = Deployments(workspace, 10)
    with pytest.raises(RuntimeError, match="Slurm jobs"):
        reset_database(deployments, scheduler, discard=True)
    scheduler.cancel(["10"])
    result = reset_database(deployments, scheduler)
    assert Path(result["archive"]).is_dir()
    assert list(Path(result["archive"]).rglob("PG_VERSION"))
    assert not data.exists()
    assert (workspace / "resources/db/postgres/keep").read_text() == "other offset"
    assert (physics / "keep").read_text() == "physics"
    data.mkdir()
    (data / "PG_VERSION").write_text("15")
    assert reset_database(deployments, scheduler, discard=True)["archive"] is None
    assert not data.exists()


def test_database_preflight_rejects_old_model_and_reset_rejects_escape(workspace):
    from gbops.deployment import preflight_database
    from gbops.maintenance import reset_database

    manifest = create_manifest(workspace)
    data = Path(manifest["database_paths"]["data_dir"])
    data.mkdir(parents=True)
    (data / "PG_VERSION").write_text("15")
    marker = Path(manifest["database_paths"]["marker"])
    marker.write_text('{"model_version":3}')
    with pytest.raises(ValueError, match="reset-db"):
        preflight_database(workspace, {}, 0, manifest["image"])
    manifest["state"] = "stopped"
    Deployments(workspace, 0).save(manifest)
    shutil.rmtree(data)
    data.symlink_to(workspace / "images", target_is_directory=True)
    with pytest.raises(ValueError, match="strictly inside"):
        reset_database(Deployments(workspace, 0), FakeSlurm(), discard=True)
    assert (workspace / "images/gammaboard/test.sif").exists()


def test_worker_deadline_is_checked_before_submission_intent(workspace):
    import time

    manifest = create_manifest(workspace)
    write_json(runtime.directory(manifest) / "control-status.json", {"end_time": time.time() + 210})
    api, scheduler = FakeApi(1), FakeSlurm()
    with pytest.raises(RuntimeError, match="ending"):
        u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert api.request_data["workers"][0]["state"] == "pending"
    assert not scheduler.submissions
    write_json(runtime.directory(manifest) / "control-status.json", {"end_time": time.time() + 500})
    u.resolve_launch_requests(manifest, "compute-a", api, scheduler)
    assert "--time=5" in scheduler.submissions[0][1]["options"]


def test_shared_control_status_avoids_per_worker_slurm_polls(workspace):
    import time

    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    runtime.heartbeat(manifest, "running")
    scheduler.lookup = Mock(side_effect=AssertionError("healthy heartbeat needs no scheduler"))
    for _ in range(10):
        assert not runtime.control_finished(manifest, scheduler)
    write_json(
        runtime.directory(manifest) / "control-status.json",
        {"updated_at": time.time() - 100, "id": manifest["id"], "state": "running"},
    )
    scheduler.lookup = Mock(
        return_value=Job(
            "10", job_name(manifest, "control"), "COMPLETED", "", tag(manifest, "control")
        )
    )
    for _ in range(10):
        assert runtime.control_finished(manifest, scheduler)
    assert scheduler.lookup.call_count == 1
    (runtime.directory(manifest) / "scheduler-status.json").unlink()
    scheduler.lookup = Mock(side_effect=SchedulerError("accounting unavailable"))
    with pytest.raises(SchedulerError):
        runtime.control_finished(manifest, scheduler)
    for _ in range(10):
        assert not runtime.control_finished(manifest, scheduler)
    assert scheduler.lookup.call_count == 1


def test_scheduler_snapshot_batches_queries():
    runner = Mock(
        return_value=subprocess.CompletedProcess(
            [], 0, "10|gb-d-control|RUNNING|compute-a|gb:d:control\n", ""
        )
    )
    scheduler = Slurm(runner)
    with scheduler.snapshot():
        for _ in range(20):
            assert scheduler.lookup("10", "2026-10-29").id == "10"
    assert runner.call_count == 1


def test_control_supervises_launcher_and_cleans_its_workers_on_exit(workspace, monkeypatch):
    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs["99"] = Job("99", "other-job", "RUNNING", "other", "")
    api = FakeApi()
    monkeypatch.setattr(u, "Api", lambda *_: api)
    process = Mock()
    process.poll.side_effect = lambda: 0 if len(scheduler.submissions) == 2 else None
    process.returncode = 0
    monkeypatch.setattr(u.subprocess, "Popen", lambda *args, **kw: process)
    assert (
        u.run_control(
            argparse_args(manifest=str(runtime.directory(manifest) / "manifest.json")), scheduler
        )
        == 0
    )
    assert len(scheduler.submissions) == 2
    assert scheduler.canceled == ["100", "101"]
    status = json.loads((runtime.directory(manifest) / "control-status.json").read_text())
    assert status["state"] == "stopped"
    assert not scheduler.jobs["99"].terminal


def test_stop_process_has_a_bounded_grace_period():
    process = Mock()
    process.poll.return_value = None
    process.wait.side_effect = [subprocess.TimeoutExpired("process", 120), 0]
    runtime.stop_process(process)
    process.terminate.assert_called_once()
    process.kill.assert_called_once()
    assert process.wait.call_args_list[0].kwargs == {"timeout": 120}


def test_prune_preserves_current_rollback_and_pending_builds(workspace):
    from gbops.maintenance import prune

    scheduler = FakeSlurm()
    manifests = []
    for i in range(3):
        image = workspace / f"images/gammaboard/revision-{i}/digest-{i}/gammaboard.sif"
        image.parent.mkdir(parents=True)
        image.write_bytes(str(i).encode())
        info = {
            "family": "gammaboard",
            "path": str(image),
            "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "build_info": {"build_id": str(i), "model_version": 7},
            "validated": True,
        }
        write_json(image.parent / "image.json", info)
        write_json(workspace / "images/gammaboard/current.json", info)
        manifest = create_manifest(workspace)
        manifest["state"] = "stopped"
        Deployments(workspace, 0).save(manifest)
        manifests.append(manifest)
    for identifier, state, job_state in [
        ("finished", "completed", "COMPLETED"),
        ("pending", "submitted", "PENDING"),
        ("unknown", "submitting", None),
    ]:
        build = workspace / "artifacts/builds" / identifier
        build.mkdir(parents=True)
        write_json(
            build / "build.json",
            {
                "id": identifier,
                "job_id": identifier if job_state else None,
                "created_at": "2026-10-29",
                "state": state,
            },
        )
        if job_state:
            scheduler.jobs[identifier] = Job(
                identifier, f"gb-build-{identifier}", job_state, "", ""
            )
    deployments = Deployments(workspace, 0)
    preview = prune(deployments, scheduler)
    first = workspace / "deployments" / manifests[0]["id"]
    assert str(first) in preview["paths"] and first.exists()
    assert str(workspace / "artifacts/builds/finished") in preview["paths"]
    assert not any("pending" in path or "unknown" in path for path in preview["paths"])
    result = prune(deployments, scheduler, apply=True)
    assert result["paths"] == preview["paths"]
    assert not first.exists()
    assert not Path(manifests[0]["image"]["path"]).exists()
    assert Path(manifests[1]["image"]["path"]).exists()
    assert Path(manifests[2]["image"]["path"]).exists()
    assert (workspace / "artifacts/builds/pending").exists()
    write_json(
        workspace / "artifacts/database-archives/saved/archive.json", {"deployment": manifests[1]}
    )
    prune(deployments, scheduler, apply=True, keep=0)
    assert not (workspace / "deployments" / manifests[1]["id"]).exists()
    assert Path(manifests[1]["image"]["path"]).exists(), (
        "database archives pin their matching image"
    )


def test_custom_capabilities_and_memory_override_survive_profile_defaults(workspace):
    manifest = create_manifest(workspace)
    spec = {
        "config": {"mem_per_cpu": "3G", "custom_feature": 1},
        "capabilities": {"custom_feature": 1},
    }
    config = u.worker_configuration(manifest, spec)
    assert config["mem_per_cpu"] == "3G" and "mem" not in config
    assert "custom_feature" not in config
    worker = {"submission_key": "key"}
    scheduler = FakeSlurm()
    u.submit_worker(manifest, "compute-a", spec, worker, scheduler, config)
    saved = json.loads((runtime.directory(manifest) / "workers/key.json").read_text())
    assert saved["capabilities"] == {"custom_feature": 1, "cpus": 1}


def test_scheduler_snapshot_batches_accounting_queries_too():
    runner = Mock(
        side_effect=[
            subprocess.CompletedProcess([], 0, "", ""),
            subprocess.CompletedProcess(
                [], 0, "10|gb-d-control|COMPLETED|compute-a|gb:d:control\n", ""
            ),
        ]
    )
    scheduler = Slurm(runner)
    with scheduler.snapshot():
        assert scheduler.lookup("10", "2026-10-29").terminal
        for identifier in ["11", "12", "13"]:
            assert scheduler.lookup(identifier, "2026-10-29") is None
    assert runner.call_count == 2


def test_stopping_launcher_records_inflight_receipt_but_stops_next_submission(workspace):
    import threading

    manifest = create_manifest(workspace)
    api, scheduler, stop = FakeApi(), FakeSlurm(), threading.Event()
    submit = scheduler.submit

    def finish_then_stop(*args, **kwargs):
        job = submit(*args, **kwargs)
        stop.set()
        return job

    scheduler.submit = finish_then_stop
    u.resolve_launch_requests(manifest, "compute-a", api, scheduler, stop=stop)
    assert [worker["state"] for worker in api.request_data["workers"]] == ["submitted", "pending"]
    assert api.request_data["workers"][0]["job_id"] == "100"
    assert len(scheduler.submissions) == 1


def test_unconfirmed_worker_cancellation_keeps_control_and_blocks_reset(workspace):
    from gbops.maintenance import reset_database

    manifest = create_manifest(workspace)
    scheduler = FakeSlurm()
    control(manifest, scheduler)
    scheduler.jobs["20"] = Job(
        "20", job_name(manifest, "worker"), "RUNNING", "compute-b", tag(manifest, "worker")
    )
    scheduler.cancel = lambda ids: scheduler.canceled.extend(ids)
    with pytest.raises(RuntimeError, match="still present"):
        u.down(u.parser().parse_args(["down", "--force", "--cancel-timeout", "1"]), scheduler)
    assert scheduler.canceled == ["20"]
    with pytest.raises(RuntimeError, match="Slurm jobs"):
        reset_database(Deployments(workspace, 0), scheduler, discard=True)


def test_reset_refuses_modified_runtime_snapshot(workspace):
    from gbops.maintenance import reset_database

    manifest = create_manifest(workspace)
    Path(manifest["runtime_config"]).write_text('[local_postgres]\ndata_dir="db/other"\n')
    with pytest.raises(ValueError, match="snapshot was modified"):
        reset_database(Deployments(workspace, 0), FakeSlurm(), discard=True)


def test_reset_refuses_active_other_offset_with_overlapping_data(workspace):
    from gbops.maintenance import reset_database

    other = create_manifest(workspace, 10)
    data = Path(other["database_paths"]["data_dir"])
    data.mkdir(parents=True)
    (data / "PG_VERSION").write_text("15")
    (workspace / "config").mkdir()
    (workspace / "config/runtime.toml").write_text('[local_postgres]\ndata_dir="db/postgres-10"\n')
    create_manifest(workspace, 0)
    scheduler = FakeSlurm()
    control(other, scheduler)
    with pytest.raises(RuntimeError, match="Slurm jobs"):
        reset_database(Deployments(workspace, 0), scheduler, discard=True)
    assert (data / "PG_VERSION").exists()
