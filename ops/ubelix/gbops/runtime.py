"""Pinned container arguments and process supervision shared by both job roles."""

from contextlib import contextmanager
import json
import os
from pathlib import Path
import signal
import subprocess
import threading
import time

from .deployment import Deployments, digest, owns, write_json

from .slurm import SHUTDOWN_SECONDS, SchedulerError


def directory(manifest):
    return Path(manifest["workspace"]) / "deployments" / manifest["id"]


def read_manifest(path):
    manifest = json.loads(Path(path).read_text())
    if manifest.get("version") != 1:
        raise ValueError("unsupported deployment manifest")
    return manifest


def runtime_command(manifest, node=None):
    args = [
        "gammaboard",
        "--runtime-config",
        manifest["runtime_config"],
        "--resource-root",
        manifest["workspace"] + "/resources",
    ]
    if node is None:
        args += ["--port-offset", str(manifest["port_offset"])]
        if not manifest["single_node"]:
            args.append("--postgres-trusted-network")
    else:
        # Explicit URLs already contain the offset. Do not apply it a second time.
        args += [
            "--database-url",
            f"postgresql://postgres@{node}:{5400 + manifest['port_offset']}/gammaboard_db",
        ]
    return args


def container_command(manifest, command, gpu=False):
    image = manifest["image"]
    for name in ("server", "runtime"):
        if digest(manifest[name + "_config"]) != manifest[name + "_digest"]:
            raise RuntimeError("deployment configuration snapshot was modified")
    if digest(image["path"]) != image["sha256"]:
        raise RuntimeError("pinned image was modified")
    env = dict(os.environ)
    env.update(
        {
            "APPTAINERENV_GAMMABOARD_DEPLOYMENT_ID": manifest["id"],
            "APPTAINERENV_TZ": "Etc/UTC",
            "APPTAINERENV_LANG": "C.UTF-8",
            "APPTAINERENV_LC_ALL": "C.UTF-8",
        }
    )
    args = ["apptainer", "exec", "--cleanenv", "--bind", manifest["workspace"]]
    if gpu:
        args.append("--nv")
    return [*args, image["path"], *command], env


@contextmanager
def stopping_on_signal():
    stop = threading.Event()
    previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    for sig in previous:
        signal.signal(sig, lambda *_: stop.set())
    try:
        yield stop
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def stop_process(process, timeout=SHUTDOWN_SECONDS, send_signal=True):
    if process.poll() is None:
        if send_signal:
            process.terminate()
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def heartbeat(manifest, state, **fields):
    write_json(
        directory(manifest) / "control-status.json",
        {"id": manifest["id"], "state": state, "updated_at": time.time(), **fields},
    )


def control_finished(manifest, slurm):
    """Healthy workers read one small heartbeat; stale-control lookups are shared."""
    root = directory(manifest)
    path = root / "control-status.json"
    status = json.loads(path.read_text()) if path.exists() else {}
    if status.get("id") == manifest["id"]:
        if status["state"] in ("stopping", "stopped"):
            return True
        if time.time() - status.get("updated_at", 0) < 90:
            return False
    deployments = Deployments(manifest["workspace"], manifest["port_offset"])
    with deployments.locked():
        cache = root / "scheduler-status.json"
        saved = json.loads(cache.read_text()) if cache.exists() else {}
        if time.time() - saved.get("checked_at", 0) < 30:
            return saved.get("terminal", False)
        try:
            job = slurm.lookup(manifest["control_job_id"], manifest["created_at"][:10])
        except SchedulerError as error:
            # Cache unknown as well: an accounting outage must not make every
            # worker query Slurm on every heartbeat check.
            write_json(cache, {"checked_at": time.time(), "terminal": False, "error": str(error)})
            raise
        if job is not None and not owns(job, manifest, "control"):
            raise RuntimeError("control job identity changed")
        terminal = job is not None and job.terminal
        write_json(cache, {"checked_at": time.time(), "terminal": terminal})
        return terminal
