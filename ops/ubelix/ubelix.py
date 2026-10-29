#!/usr/bin/env python3
"""Deployment-scoped cluster control. Run on a login node with Python 3.11.8+."""

from __future__ import annotations

import sys

if sys.version_info < (3, 11, 8):
    raise SystemExit(
        f"GammaBoard's UBELIX helper requires Python 3.11.8+; running {sys.version.split()[0]}.\n"
        "Run 'module load Python', then check 'python3 --version'. "
        "Use 'module spider Python' to find a suitable version if needed.\n"
        "Load the same module in batch jobs via ~/.config/gammaboard/slurm.env."
    )

import argparse
import base64
from contextlib import contextmanager, suppress
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import urllib.error
import urllib.request
import uuid
from socket import gethostname

from gbops.deployment import Deployments, tag, write_json, job_name, owns
from gbops.slurm import (
    Slurm,
    SchedulerError,
    options as sbatch_args_from_config,
    time_seconds,
    wait_absent,
    overlay_resources,
    SBATCH_OPTIONS,
    SIGNAL_MARGIN_SECONDS,
    SHUTDOWN_SECONDS,
)
from gbops.runtime import (
    container_command,
    runtime_command,
    read_manifest,
    stopping_on_signal,
    stop_process,
    heartbeat,
    control_finished,
    directory,
)
from gbops.maintenance import reset_database
from gbops import builds, settings


def opener():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}))


def shifted_port(base, offset):
    value = base + offset
    if not 0 < value <= 65535:
        raise ValueError("port out of range")
    return value


def tunnel_command(node, local_port, offset):
    target = (
        settings.local_config().get("ssh_target")
        or f"{os.environ.get('USER', '')}@submit03.unibe.ch"
    )
    return f"ssh -N -L {local_port}:{node}:{shifted_port(8080, offset)} {target}"


class Api:
    def __init__(self, manifest, node, password=""):
        self.base = f"http://{node}:{8080 + manifest['port_offset']}/api"
        credential = directory(manifest) / "launcher-auth.json"
        self.password = password or (
            json.loads(credential.read_text())["password"] if credential.exists() else ""
        )
        self.cookie = None
        health = self.request("/health", method="GET")
        if (
            health.get("deployment_id") != manifest["id"]
            or health.get("build_id") != manifest["image"]["build_info"]["build_id"]
        ):
            raise RuntimeError("API identity does not match the selected deployment")
        self.login()

    def login(self):
        request = urllib.request.Request(
            self.base + "/auth/login",
            data=json.dumps({"password": self.password}).encode(),
            headers={"Content-Type": "application/json"},
        )
        with opener().open(request, timeout=10) as response:
            self.cookie = response.headers.get("Set-Cookie", "").split(";")[0]

    def request(self, path, payload=None, method="POST"):
        headers = {"Content-Type": "application/json"}
        if self.cookie:
            headers["Cookie"] = self.cookie
        request = urllib.request.Request(
            self.base + path,
            data=None if method == "GET" else json.dumps(payload or {}).encode(),
            headers=headers,
            method=method,
        )
        with opener().open(request, timeout=10) as response:
            return json.load(response)


def launch_groups_for_request(request):
    groups = request.get("args", {}).get("groups")
    if not isinstance(groups, list) or not groups:
        raise ValueError("external launches require nonempty args.groups; recreate old requests")
    specs = {}
    for group in groups:
        names = group.get("node_names")
        count = group.get("count")
        if (
            not isinstance(count, int)
            or isinstance(count, bool)
            or count < 1
            or not isinstance(names, list)
            or len(names) != count
        ):
            raise ValueError("each group requires its reserved node_names and positive count")
        config, caps = group.get("config"), group.get("capabilities")
        if not isinstance(config, dict) or not isinstance(caps, dict):
            raise ValueError("group requires server-normalized config and capabilities")
        if any(key in config for key in ("gpu", "cores", "cpus", "nr_cores", "cpus-per-task")):
            raise ValueError("launcher received a noncanonical resource configuration")
        for name in names:
            if not isinstance(name, str) or not name or name in specs:
                raise ValueError("invalid/duplicate reserved worker name")
            specs[name] = {
                "node_name": name,
                "config": config,
                "capabilities": caps,
                "max_start_failures": group.get("max_start_failures", 3),
            }
    return specs


class Lease:
    def __init__(self, api, request):
        self.api, self.request = api, request
        self.stop, self.lost = threading.Event(), threading.Event()

    def heartbeat(self):
        while not self.stop.wait(20):
            try:
                self.api.request(
                    f"/node-launch-requests/{self.request['id']}/lease",
                    {"lease_token": self.request["lease_token"]},
                )
            except Exception:
                self.lost.set()
                return

    def __enter__(self):
        self.thread = threading.Thread(target=self.heartbeat, daemon=True)
        self.thread.start()
        return self

    def check(self):
        if self.lost.is_set():
            raise RuntimeError("launcher lease could not be renewed; reconciliation required")

    def update(self, name, state, job_id=None, error=None):
        self.check()
        self.api.request(
            f"/node-launch-requests/{self.request['id']}/progress",
            {
                "lease_token": self.request["lease_token"],
                "node_name": name,
                "state": state,
                "job_id": job_id,
                "error": error,
            },
        )

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(timeout=12)
        with suppress(Exception):
            self.api.request(
                f"/node-launch-requests/{self.request['id']}/lease",
                {"lease_token": self.request["lease_token"], "release": True},
            )


def worker_configuration(manifest, spec):
    directory = Path(manifest["workspace"]) / "deployments" / manifest["id"]
    resources = {key: value for key, value in spec["config"].items() if key in SBATCH_OPTIONS}
    config = overlay_resources(manifest["profiles"]["worker"], resources)
    status_file = directory / "control-status.json"
    status = json.loads(status_file.read_text()) if status_file.exists() else {}
    if status.get("end_time"):
        remaining = int(status["end_time"] - time.time()) - SIGNAL_MARGIN_SECONDS
        if remaining < 60:
            raise RuntimeError("control allocation is ending; worker requests remain pending")
        config["time"] = str(max(1, min(time_seconds(config["time"]), remaining) // 60))
    if time_seconds(config["time"]) <= SIGNAL_MARGIN_SECONDS:
        raise RuntimeError("worker time must exceed the shutdown margin; control may be ending")
    sbatch_args_from_config(config)
    return config


def submit_worker(manifest, node, spec, worker, slurm, config):
    directory = Path(manifest["workspace"]) / "deployments" / manifest["id"]
    recipe = directory / "workers" / f"{worker['submission_key']}.json"
    caps = {**spec["capabilities"], "cpus": int(config["cpus_per_task"])}
    write_json(
        recipe,
        {
            "manifest": str(directory / "manifest.json"),
            "database_node": node,
            "control_job_id": manifest.get("control_job_id"),
            **spec,
            "config": config,
            "capabilities": caps,
        },
    )
    env = dict(
        os.environ,
        GAMMABOARD_WORKER_SPEC=str(recipe),
        GAMMABOARD_MANIFEST=str(directory / "manifest.json"),
    )
    env.pop("GAMMABOARD_ADMIN_PASSWORD", None)
    return slurm.submit(
        directory / "launcher/worker.sbatch",
        name=job_name(manifest, worker["submission_key"]),
        comment=tag(manifest, worker["submission_key"]),
        workspace=manifest["workspace"],
        env=env,
        options=[*sbatch_args_from_config(config), f"--signal=B:TERM@{SIGNAL_MARGIN_SECONDS}"],
    )


@contextmanager
def submission_guard(manifest):
    deployments = Deployments(manifest["workspace"], manifest["port_offset"])
    with deployments.locked():
        current = deployments.load()
        if current["id"] != manifest["id"] or current["state"] in ("stopping", "stopped"):
            raise RuntimeError("deployment is stopping or has been replaced")
        yield


def resolve_launch_requests(manifest, node, api, slurm, owner=None, stop=None):
    with submission_guard(manifest):
        pass
    with slurm.snapshot():
        return reconcile_launch_requests(manifest, node, api, slurm, owner, stop)


def reconcile_launch_requests(manifest, node, api, slurm, owner=None, stop=None):
    """One fair pass. A missing ambiguous submission is never automatically repeated."""
    owner = owner or uuid.uuid4().hex
    seen = set()
    while True:
        if stop is not None and stop.is_set():
            return len(seen)
        request = api.request("/node-launch-requests/claim-external", {"launcher_id": owner}).get(
            "request"
        )
        if request is None:
            return len(seen)
        with Lease(api, request) as lease:
            if request["id"] in seen:
                return len(seen)
            seen.add(request["id"])
            specs = launch_groups_for_request(request)
            for worker in request["workers"]:
                if stop is not None and stop.is_set():
                    return len(seen)
                lease.check()
                name, state = worker["node_name"], worker["state"]
                if state in ("connected", "failed", "canceled"):
                    continue
                if worker.get("registered"):
                    lease.update(name, "connected", worker.get("job_id"))
                    continue
                if state == "pending":
                    with submission_guard(manifest):
                        spec = specs[name]
                        # Validate all local inputs before committing submission intent.
                        config = worker_configuration(manifest, spec)
                        lease.update(name, "submitting")
                        job_id = submit_worker(manifest, node, spec, worker, slurm, config)
                        lease.update(name, "submitted", job_id)
                        print(f"request={request['id']} node={name} job={job_id}")
                elif state == "submitting":
                    matches = slurm.find(
                        tag(manifest, worker["submission_key"]), manifest["created_at"][:10]
                    )
                    if len(matches) == 1:
                        job = matches[0]
                        lease.update(name, "submitted", job.id)
                        if job.terminal:
                            lease.update(
                                name,
                                "failed",
                                job.id,
                                f"Slurm job ended before registration: {job.state}",
                            )
                    else:
                        print(
                            f"request={request['id']} node={name}: submission uncertain ({len(matches)} matching jobs); reconcile before retry",
                            file=sys.stderr,
                        )
                elif state == "submitted":
                    job = slurm.lookup(worker["job_id"], manifest["created_at"][:10])
                    if job and not owns(job, manifest, worker["submission_key"]):
                        raise RuntimeError(
                            "scheduler job identity does not match the worker reservation"
                        )
                    if job and job.terminal:
                        lease.update(
                            name,
                            "failed",
                            job.id,
                            f"Slurm job ended before registration: {job.state}",
                        )


def control_job(manifest, slurm):
    if not manifest.get("control_job_id"):
        return None
    job = slurm.lookup(manifest["control_job_id"], manifest["created_at"][:10])
    if job and not owns(job, manifest, "control"):
        raise RuntimeError("control job belongs to another deployment")
    return job


def require_deployment(args):
    deployments = Deployments(settings.workspace_root(), args.port_offset)
    manifest = deployments.load()
    if not manifest:
        raise RuntimeError("no deployment recorded for this workspace/offset")
    return deployments, manifest


def up(args, slurm):
    deployments = Deployments(settings.workspace_root(), args.port_offset)
    config = deployments.configuration(args.single_node, args.local_port, args.time)
    with deployments.locked():
        builds.require_no_build(deployments.workspace, slurm)
        image = deployments.image()
        manifest = deployments.load()
        if args.fresh_db:
            reset_database(deployments, slurm)
        if manifest and manifest["state"] == "stopped":
            manifest = None
        if manifest:
            if manifest.get("control_job_id"):
                job = control_job(manifest, slurm)
                if job is None:
                    raise RuntimeError(
                        "control status is unknown; refusing to create another deployment"
                    )
                if job.terminal:
                    if any(owns(worker, manifest) for worker in slurm.active()):
                        raise RuntimeError(
                            "old deployment still has active jobs; run down before replacing it"
                        )
                    manifest = None
            elif manifest["control_submission"] == "submitting":
                matches = slurm.find(tag(manifest, "control"), manifest["created_at"][:10])
                if len(matches) != 1:
                    raise RuntimeError(
                        "control submission uncertain; use resolve-control after checking Slurm"
                    )
                manifest["control_job_id"] = matches[0].id
                manifest["control_submission"] = "submitted"
                deployments.save(manifest)
                if matches[0].terminal:
                    if any(owns(worker, manifest) for worker in slurm.active()):
                        raise RuntimeError(
                            "old deployment still has active jobs; run down before replacing it"
                        )
                    manifest = None
        if manifest and manifest["state"] == "stopping":
            raise RuntimeError("deployment is stopping; wait for its jobs to finish")
        if manifest is not None and (
            manifest["single_node"] != args.single_node
            or manifest["config_hash"] != config["hash"]
            or manifest["image"]["sha256"] != image["sha256"]
        ):
            raise RuntimeError(
                "active deployment has different image/configuration; stop it before starting the replacement"
            )
        if manifest and args.resume_workers and not manifest["resume_workers"]:
            raise RuntimeError(
                "--resume-workers is a startup setting; stop the active deployment before enabling it"
            )
        if manifest is None:
            manifest = deployments.create(
                image, config, args.single_node, args.resume_workers, args.admin_password
            )
        if manifest["control_submission"] == "prepared":
            manifest["control_submission"] = "submitting"
            deployments.save(manifest)
            for name in ("control", "workers", "build"):
                (deployments.workspace / "logs/slurm" / name).mkdir(parents=True, exist_ok=True)
            env = dict(
                os.environ,
                GAMMABOARD_MANIFEST=str(deployments.root / manifest["id"] / "manifest.json"),
            )
            manifest["control_job_id"] = slurm.submit(
                deployments.root / manifest["id"] / "launcher/control.sbatch",
                name=job_name(manifest, "control"),
                comment=tag(manifest, "control"),
                workspace=deployments.workspace,
                env=env,
                options=[
                    *sbatch_args_from_config(manifest["profiles"]["control"]),
                    f"--signal=B:TERM@{SIGNAL_MARGIN_SECONDS}",
                ],
            )
            manifest["control_submission"] = "submitted"
            deployments.save(manifest)
    print(f"deployment={manifest['id']} control_job={manifest['control_job_id']}")
    node, api = wait_ready(manifest, slurm, args.startup_timeout, args.admin_password)
    with deployments.locked():
        current = deployments.load()
        if current["id"] != manifest["id"] or current["state"] in ("stopping", "stopped"):
            raise RuntimeError("deployment stopped or was replaced during startup")
        current["state"] = "running"
        deployments.save(current)
    tunnel = tunnel_command(node, args.local_port, args.port_offset)
    print(f"tunnel={tunnel}")
    if args.copy and sys.stdout.isatty():
        sys.stdout.write("\033]52;c;" + base64.b64encode(tunnel.encode()).decode() + "\a")


def wait_ready(manifest, slurm, timeout, password):
    deadline, last = time.monotonic() + timeout, "not started"
    while time.monotonic() < deadline:
        try:
            job = control_job(manifest, slurm)
            if job and job.terminal:
                raise RuntimeError(f"control job ended: {job.state}")
            if job and job.node:
                try:
                    api = Api(manifest, job.node, password)
                    status_file = directory(manifest) / "control-status.json"
                    status = json.loads(status_file.read_text()) if status_file.exists() else {}
                    if status.get("launcher_last_success") and not status.get("launcher_error"):
                        return job.node, api
                    last = status.get("launcher_error") or "managed request launcher is starting"
                except (OSError, urllib.error.URLError) as error:
                    last = str(error)
        except SchedulerError as error:
            last = str(error)
        time.sleep(2)
    raise RuntimeError(
        f"deployment not ready within timeout ({last}); it remains recorded; down can cancel a pending job"
    )


def down(args, slurm):
    deployments, _ = require_deployment(args)
    with deployments.locked():
        manifest = deployments.load()
        if not manifest.get("control_job_id"):
            if manifest["control_submission"] == "prepared":
                manifest["state"] = "stopped"
                deployments.save(manifest)
                return
            matches = slurm.find(tag(manifest, "control"), manifest["created_at"][:10])
            if len(matches) != 1:
                raise RuntimeError("control submission uncertain; use resolve-control")
            manifest["control_job_id"] = matches[0].id
        job = control_job(manifest, slurm)
        if job is None:
            raise RuntimeError("control status unknown; no jobs canceled")
        manifest["state"] = "stopping"
        deployments.save(manifest)
    worker_owned = lambda worker: owns(worker, manifest) and worker.id != job.id
    if not job.terminal and job.state != "PENDING" and not args.force:
        if not job.node:
            raise RuntimeError(
                "control has no reachable node; use down --force after inspecting status"
            )
        try:
            Api(manifest, job.node, args.admin_password).request("/admin/workers/suspend")
        except (OSError, RuntimeError, urllib.error.URLError) as error:
            raise RuntimeError(
                "API shutdown failed; use down --force to cancel verified owned jobs"
            ) from error
        try:
            wait_absent(slurm, worker_owned, args.worker_timeout)
        except RuntimeError:
            pass
    slurm.cancel([worker.id for worker in slurm.active() if worker_owned(worker)])
    # Keep PostgreSQL alive until Slurm confirms the worker allocations are gone.
    wait_absent(slurm, worker_owned, args.cancel_timeout)
    if not job.terminal:
        slurm.cancel([job.id])
    wait_absent(slurm, lambda candidate: owns(candidate, manifest), args.cancel_timeout)
    final = control_job(manifest, slurm)
    if final is None or not final.terminal:
        raise RuntimeError("control termination not confirmed; retry down")
    with deployments.locked():
        manifest = deployments.by_id(manifest["id"])
        manifest["state"] = "stopped"
        deployments.save(manifest)
    print(f"stopped deployment={manifest['id']}")


def watch(manifest, node, api, slurm, interval, once=False):
    delay = interval
    while True:
        try:
            job = control_job(manifest, slurm)
            if job and job.terminal:
                return
            if job is None:
                raise SchedulerError("control status unknown")
            if not manifest["single_node"]:
                resolve_launch_requests(manifest, node, api, slurm)
            if once:
                return
            delay = interval
        except urllib.error.HTTPError as error:
            if error.code == 401:
                api.login()
            else:
                print(f"launcher retry: {error}", file=sys.stderr)
            if once:
                raise
            delay = min(60, max(interval, delay * 2))
        except (OSError, SchedulerError, RuntimeError) as error:
            if once:
                raise
            print(f"launcher retry: {error}", file=sys.stderr)
            delay = min(60, max(interval, delay * 2))
        time.sleep(delay)


def run_control(args, slurm):
    manifest = read_manifest(args.manifest)
    manifest["control_job_id"] = os.environ.get("SLURM_JOB_ID") or manifest.get("control_job_id")
    command = runtime_command(manifest) + ["deploy", "--server-config", manifest["server_config"]]
    if manifest["resume_workers"]:
        command.append("--resume-workers")
    for origin in manifest["origins"]:
        command += ["--allowed-origin", origin]
    control_resources = manifest["profiles"]["control"]
    gpu = bool(control_resources.get("gpus")) or any(
        (part == "gpu" or part.startswith("gpu:"))
        for part in str(control_resources.get("gres", "")).split(",")
    )
    command, env = container_command(manifest, command, gpu=gpu)
    password = json.loads((directory(manifest) / "launcher-auth.json").read_text())["password"]
    end_time = int(os.environ.get("SLURM_JOB_END_TIME", "0")) or None
    with stopping_on_signal() as stop:
        process = subprocess.Popen(command, env=env)
        launcher_status = {"launcher_error": None, "launcher_last_success": None}

        def launcher():
            api = None
            while not stop.is_set():
                try:
                    if not manifest["single_node"]:
                        if api is None:
                            api = Api(manifest, gethostname(), password)
                        resolve_launch_requests(manifest, gethostname(), api, slurm, stop=stop)
                    launcher_status.update(launcher_error=None, launcher_last_success=time.time())
                except (OSError, ValueError, RuntimeError, urllib.error.URLError) as error:
                    launcher_status["launcher_error"] = str(error)
                    print(f"launcher retry: {error}", file=sys.stderr)
                    api = None
                if stop.wait(5):
                    break

        thread = threading.Thread(target=launcher, daemon=True)
        thread.start()
        try:
            while process.poll() is None and not stop.is_set():
                if not thread.is_alive():
                    raise RuntimeError("managed request launcher exited")
                heartbeat(manifest, "running", end_time=end_time, **launcher_status)
                stop.wait(2)
        finally:
            stop.set()
            heartbeat(manifest, "stopping", end_time=end_time)
            started = time.monotonic()
            if process.poll() is None:
                process.terminate()
            # Fence every launcher before the final scheduler cleanup. An accepted
            # in-flight submission completes under this same short lock.
            deployments = Deployments(manifest["workspace"], manifest["port_offset"])
            with deployments.locked():
                saved = deployments.by_id(manifest["id"])
                saved["state"] = "stopping"
                deployments.save(saved)
            stop_process(
                process,
                timeout=max(0, SHUTDOWN_SECONDS - (time.monotonic() - started)),
                send_signal=False,
            )
            heartbeat(manifest, "stopped", end_time=end_time)
            # Unexpected control exit must not strand queued workers.
            try:
                slurm.cancel(
                    [
                        job.id
                        for job in slurm.active()
                        if owns(job, manifest) and job.id != manifest.get("control_job_id")
                    ]
                )
            except SchedulerError as error:
                print(f"worker cleanup requires down --force: {error}", file=sys.stderr)
            thread.join(timeout=1)
    return process.returncode


def run_worker(args, slurm):
    spec = json.loads(Path(args.spec).read_text())
    manifest = read_manifest(spec["manifest"])
    manifest["control_job_id"] = spec.get("control_job_id") or manifest.get("control_job_id")
    caps = spec["capabilities"]
    command = runtime_command(manifest, spec["database_node"]) + [
        "node",
        "run",
        "--name",
        spec["node_name"],
        "--max-start-failures",
        str(spec["max_start_failures"]),
    ]
    for name, count in sorted(caps.items()):
        command += ["--capability", f"{name}={count}"]
    command, env = container_command(manifest, command, gpu=caps.get("gpu", 0) > 0)
    with stopping_on_signal() as stop:
        worker = subprocess.Popen(command, env=env)
        try:
            while worker.poll() is None and not stop.wait(5):
                try:
                    if control_finished(manifest, slurm):
                        stop.set()
                except (OSError, ValueError, SchedulerError) as error:
                    print(
                        f"control status unavailable; keeping worker alive: {error}",
                        file=sys.stderr,
                    )
        finally:
            stop_process(worker)
    return worker.returncode


def resolve_control(args, slurm):
    deployments, manifest = require_deployment(args)
    with deployments.locked():
        manifest = deployments.load()
        if manifest.get("control_job_id"):
            raise ValueError("control already has a recorded job")
        matches = slurm.find(tag(manifest, "control"), manifest["created_at"][:10])
        if args.job_id:
            job = slurm.lookup(args.job_id, manifest["created_at"][:10])
            if not job or not owns(job, manifest, "control"):
                raise ValueError("job identity does not match this deployment")
            manifest["control_job_id"], manifest["control_submission"] = job.id, "submitted"
        else:
            if matches:
                raise ValueError("matching Slurm jobs exist; cannot confirm absence")
            manifest["control_submission"] = "prepared"
        deployments.save(manifest)


def resolve_submission(args, slurm):
    deployments, _ = require_deployment(args)
    with deployments.locked():
        manifest = deployments.load()
        if manifest["state"] in ("stopping", "stopped"):
            raise RuntimeError("deployment is stopping")
        return repair_submission(args, slurm, manifest)


def repair_submission(args, slurm, manifest):
    job = control_job(manifest, slurm)
    if not job or job.terminal or not job.node:
        raise RuntimeError("selected deployment is not running")
    api = Api(manifest, job.node, args.admin_password)
    request = api.request(
        "/node-launch-requests/claim-external",
        {"launcher_id": uuid.uuid4().hex, "request_id": args.request_id},
    ).get("request")
    if not request:
        raise RuntimeError(
            "request is finished or another launcher holds its lease; stop the watcher and wait for lease expiry"
        )
    with Lease(api, request) as lease:
        worker = next((w for w in request["workers"] if w["node_name"] == args.node), None)
        if not worker or worker["state"] != "submitting" or worker.get("registered"):
            raise ValueError("only an unregistered ambiguous submission can be resolved")
        expected = tag(manifest, worker["submission_key"])
        if args.job_id:
            found = slurm.lookup(args.job_id, manifest["created_at"][:10])
            if not found or not owns(found, manifest, worker["submission_key"]):
                raise ValueError("job does not match this worker submission")
            lease.update(args.node, "submitted", found.id)
            if found.terminal:
                lease.update(
                    args.node, "failed", found.id, "resolved job already ended before registration"
                )
        else:
            if slurm.find(expected, manifest["created_at"][:10]):
                raise ValueError("matching Slurm jobs exist; refusing retry")
            lease.update(args.node, "pending", error="operator confirmed no submission")
    print("submission resolved; the watcher can continue")


def main(argv=None):
    args = parser().parse_args(argv)
    slurm = Slurm()
    try:
        if args.command == "run-control":
            return run_control(args, slurm)
        if args.command == "run-build":
            return builds.execute(args.record, slurm)
        if args.command == "run-worker":
            return run_worker(args, slurm)
        args.admin_password = getattr(args, "admin_password", "") or settings.local_config().get(
            "admin_password", ""
        )
        if args.command == "up":
            up(args, slurm)
        elif args.command == "down":
            down(args, slurm)
        elif args.command == "resolve-control":
            resolve_control(args, slurm)
        elif args.command == "resolve-submission":
            resolve_submission(args, slurm)
        elif args.command == "build":
            result = builds.submit(settings.workspace_root(), args, slurm)
            print(
                json.dumps(
                    {key: result[key] for key in ("id", "job_id", "family", "revision", "state")},
                    indent=2,
                )
            )
        elif args.command == "reset-db":
            deployments = Deployments(settings.workspace_root(), args.port_offset)
            with deployments.locked():
                result = reset_database(deployments, slurm, args.discard)
            print(json.dumps(result, indent=2))
        else:
            _, manifest = require_deployment(args)
            job = control_job(manifest, slurm)
            if args.command == "status":
                print(
                    json.dumps(
                        {
                            "deployment": manifest["id"],
                            "control": None if job is None else job.__dict__,
                            "state": manifest["state"],
                            "image": manifest["image"]["path"],
                            "image_sha256": manifest["image"]["sha256"],
                            "build_id": manifest["image"]["build_info"]["build_id"],
                            "supervisor": json.loads(
                                (directory(manifest) / "control-status.json").read_text()
                            )
                            if (directory(manifest) / "control-status.json").exists()
                            else None,
                            "workers": [
                                j.__dict__
                                for j in slurm.active()
                                if owns(j, manifest) and j.id != manifest["control_job_id"]
                            ],
                        },
                        indent=2,
                    )
                )
                return
            if not job or job.terminal or not job.node:
                raise RuntimeError("selected deployment is not running")
            api = Api(manifest, job.node, args.admin_password)
            if args.command == "submit-workers":
                from gbops.deployment import toml_dump

                config = dict(args.capabilities)
                toml = (
                    f"[[groups]]\ncount={args.count}\nname_prefix={json.dumps(args.prefix)}\nmax_start_failures={args.max_start_failures}\n[groups.config]\n"
                    + toml_dump(config)
                )
                print(json.dumps(api.request("/node-launch-requests", {"toml": toml})))
            watch(
                manifest,
                job.node,
                api,
                slurm,
                args.poll_seconds,
                once=args.command == "submit-workers" or args.once,
            )
    except KeyboardInterrupt:
        print("launcher stopped; recorded submissions remain recoverable")
    except (OSError, ValueError, RuntimeError, urllib.error.URLError) as error:
        raise SystemExit(str(error)) from error


def nonnegative(value):
    n = int(value)
    if n < 0:
        raise argparse.ArgumentTypeError("must be nonnegative")
    return n


def positive(value):
    n = int(value)
    if n <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return n


def port(value):
    n = positive(value)
    if n > 65535:
        raise argparse.ArgumentTypeError("port out of range")
    return n


def offset(value):
    n = int(value)
    if not 0 <= n <= 65535 - 8080:
        raise argparse.ArgumentTypeError("port offset out of range")
    return n


def parse_capability(value):
    key, separator, raw = value.partition("=")
    if not separator or not key:
        raise argparse.ArgumentTypeError("capability must use KEY=VALUE")
    return key, positive(raw)


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    commands = p.add_subparsers(dest="command", required=True)
    for name in (
        "up",
        "down",
        "status",
        "submit-workers",
        "watch-requests",
        "resolve-control",
        "resolve-submission",
        "reset-db",
    ):
        cmd = commands.add_parser(name)
        cmd.add_argument("--port-offset", type=offset, default=0)
        cmd.add_argument("--admin-password", default="")
        if name == "up":
            cmd.add_argument("--time", help="override the control profile walltime")
            cmd.add_argument(
                "--fresh-db",
                action="store_true",
                help="archive the stopped database before creating a fresh one",
            )
            cmd.add_argument("--single-node", action="store_true")
            cmd.add_argument("--resume-workers", action="store_true")
            cmd.add_argument("--local-port", type=port, default=8080)
            cmd.add_argument("--startup-timeout", type=positive, default=180)
            cmd.add_argument("--copy", action="store_true")
        elif name == "down":
            cmd.add_argument("--worker-timeout", type=nonnegative, default=60)
            cmd.add_argument(
                "--force",
                action="store_true",
                help="skip unavailable API and cancel only verified owned jobs",
            )
            cmd.add_argument("--cancel-timeout", type=positive, default=180)
        elif name in ("submit-workers", "watch-requests"):
            cmd.add_argument("--poll-seconds", type=positive, default=5)
            cmd.add_argument("--once", action="store_true")
            if name == "submit-workers":
                cmd.add_argument("--count", type=positive, required=True)
                cmd.add_argument("--prefix", default="w")
                cmd.add_argument("--max-start-failures", type=positive, default=3)
                cmd.add_argument(
                    "--capability",
                    dest="capabilities",
                    action="append",
                    default=[],
                    type=parse_capability,
                )
        elif name == "reset-db":
            cmd.add_argument(
                "--discard",
                action="store_true",
                help="delete the stopped database instead of archiving it",
            )
        elif name in ("resolve-control", "resolve-submission"):
            if name == "resolve-submission":
                cmd.add_argument("request_id", type=positive)
                cmd.add_argument("node")
            choice = cmd.add_mutually_exclusive_group(required=True)
            choice.add_argument("--job-id")
            choice.add_argument(
                "--not-submitted",
                action="store_true",
                help="confirm, after checking Slurm, that no job was submitted",
            )
    build = commands.add_parser("build").add_subparsers(dest="build_kind", required=True)
    for name in ("gammaboard", "gammaloop", "apptainer"):
        cmd = build.add_parser(name)
        if name == "apptainer":
            cmd.add_argument("output")
            cmd.add_argument("def_file")
    commands.add_parser("run-build").add_argument("--record", required=True)
    commands.add_parser("run-control").add_argument("--manifest", required=True)
    commands.add_parser("run-worker").add_argument("--spec", required=True)
    return p


if __name__ == "__main__":
    sys.exit(main())
