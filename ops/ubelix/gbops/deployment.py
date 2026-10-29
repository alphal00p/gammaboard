from __future__ import annotations

from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import tomllib
import uuid

from . import settings
from .slurm import options, overlay_resources, time_seconds, SIGNAL_MARGIN_SECONDS, SHUTDOWN_SECONDS


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def write_json(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary = tempfile.mkstemp(dir=path.parent, prefix=".writing-")
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def merge(base, override):
    result = dict(base)
    for key, value in override.items():
        result[key] = (
            merge(result[key], value)
            if isinstance(value, dict) and isinstance(result.get(key), dict)
            else value
        )
    return result


def toml_dump(value):
    def scalar(item):
        if isinstance(item, dict):
            return "{" + ", ".join(json.dumps(k) + " = " + scalar(v) for k, v in item.items()) + "}"
        if isinstance(item, list):
            return "[" + ", ".join(map(scalar, item)) + "]"
        return json.dumps(item, ensure_ascii=False, allow_nan=False)

    lines = []

    def table(mapping, path):
        if path:
            lines.append("[" + ".".join(json.dumps(part) for part in path) + "]")
        for key, item in mapping.items():
            if not isinstance(item, dict):
                lines.append(json.dumps(key) + " = " + scalar(item))
        for key, item in mapping.items():
            if isinstance(item, dict):
                lines.append("")
                table(item, [*path, key])

    table(value, [])
    return "\n".join(lines) + "\n"


def cluster_config():
    data = tomllib.loads((settings.OPS_ROOT / "config/cluster.toml").read_text())
    for name, override in settings.local_config().get("cluster", {}).items():
        data[name] = overlay_resources(data.get(name, {}), override)
    if data.keys() != {"control", "worker", "build"}:
        raise ValueError("cluster config requires control, worker and build profiles")
    if any(key in data["worker"] for key in ("gpus", "gres")):
        raise ValueError(
            "declare worker GPU resources in request groups so capabilities stay canonical"
        )
    for profile in data.values():
        options(profile)
        time_seconds(profile["time"])
        if type(profile.get("cpus_per_task")) is not int or profile["cpus_per_task"] < 1:
            raise ValueError("cpus_per_task must be a positive integer")
    return data


def database_paths(workspace, runtime, offset):
    root = Path(workspace).resolve() / "resources"
    config = runtime.get("local_postgres", {})
    result = {}
    for key, default in [
        ("data_dir", "db/postgres"),
        ("socket_dir", "db/socket"),
        ("log_file", "db/logfile"),
    ]:
        path = Path(config.get(key, default))
        if not path.is_absolute():
            path = root / path
        # Match RuntimeContext's offset handling, including logs inside PGDATA.
        original_data = Path(config.get("data_dir", "db/postgres"))
        if not original_data.is_absolute():
            original_data = root / original_data
        if offset:
            shifted_data = original_data.with_name(original_data.name + f"-{offset}")
            path = (
                shifted_data / path.relative_to(original_data)
                if key == "log_file" and path.is_relative_to(original_data)
                else path.with_name(path.name + f"-{offset}")
            )
        result[key] = path
    result["marker"] = result["data_dir"].with_name(result["data_dir"].name + ".gammaboard.json")
    return result


def preflight_database(workspace, runtime, offset, image):
    paths = database_paths(workspace, runtime, offset)
    data = paths["data_dir"]
    if data.exists() and any(data.iterdir()):
        marker = paths["marker"]
        saved = json.loads(marker.read_text()) if marker.exists() else {}
        if saved.get("model_version") != image["build_info"]["model_version"]:
            raise ValueError(
                "existing database has an unknown/incompatible model; stop it and run reset-db (or reset-db --discard) first"
            )
    return paths


class Deployments:
    def __init__(self, workspace, offset):
        self.workspace = Path(workspace).resolve()
        self.offset = offset
        if not 0 <= offset <= 65535 - 8080:
            raise ValueError("port offset out of range")
        self.root = self.workspace / "deployments"
        self.slot = hashlib.sha256(f"{self.workspace}\0{offset}".encode()).hexdigest()[:16]
        self.pointer = self.root / f"active-{self.slot}.json"

    @contextmanager
    def locked(self):
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        fd = os.open(self.root / "workspace.lock", os.O_CREAT | os.O_RDWR, 0o600)
        with os.fdopen(fd, "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            yield

    def load(self):
        if not self.pointer.exists():
            return None
        record = json.loads(self.pointer.read_text())
        return self.by_id(record["id"])

    def by_id(self, identifier):
        if not isinstance(identifier, str) or not identifier.isalnum():
            raise ValueError("invalid deployment id")
        manifest = json.loads((self.root / identifier / "manifest.json").read_text())
        if manifest["workspace"] != str(self.workspace) or manifest["port_offset"] != self.offset:
            raise ValueError("deployment does not belong to this workspace/offset")
        return manifest

    def save(self, manifest):
        write_json(self.root / manifest["id"] / "manifest.json", manifest)

    def image(self):
        metadata = self.workspace / "images/gammaboard.json"
        if not metadata.exists():
            raise ValueError("no built image; run build gammaboard and wait for completion")
        info = json.loads(metadata.read_text())
        image = self.workspace / "images/gammaboard.sif"
        if digest(image) != info["sha256"]:
            raise ValueError("image digest does not match its manifest")
        if not info.get("validated") or not info.get("build_info", {}).get("build_id"):
            raise ValueError("image has no validated build identity")
        if type(info["build_info"].get("model_version")) is not int:
            raise ValueError("image does not report its database model")
        return {**info, "path": str(image)}

    def configuration(self, single, local_port, control_time=None):
        local = settings.local_config()
        base = settings.OPS_ROOT / "config/server.toml"
        server = merge(tomllib.loads(base.read_text()), local.get("server", {}))
        build_dir = server.get("frontend", {}).get("build_dir")
        if build_dir and not Path(build_dir).is_absolute():
            server["frontend"]["build_dir"] = str(settings.local_path(build_dir))
        runtime = local.get("runtime", {})
        # Ports and launch mode belong to the deployment, and are supplied explicitly.
        if (
            server.get("api_port", 4000) != 4000
            or server.get("frontend", {}).get("port", 8080) != 8080
        ):
            raise ValueError("use --port-offset instead of overriding base server ports")
        if "url" in runtime.get("database", {}):
            raise ValueError(
                "cluster deployments manage their database URL; configure PostgreSQL tuning only"
            )
        server["allow_local_node_spawn"] = single
        origins = [f"http://localhost:{local_port}", f"http://127.0.0.1:{local_port}"]
        profiles = cluster_config()
        if control_time is not None:
            profiles["control"]["time"] = control_time
        if time_seconds(profiles["control"]["time"]) <= SIGNAL_MARGIN_SECONDS:
            raise ValueError("control time must exceed its 180-second shutdown margin")
        if (
            server.get("cleanup", {}).get("sampler_drain_timeout_seconds", 60)
            + server.get("cleanup", {}).get("node_stop_timeout_seconds", 15)
            > SHUTDOWN_SECONDS
        ):
            raise ValueError(
                "control cleanup must fit within the 120-second process shutdown budget"
            )
        data = {"server": server, "runtime": runtime, "origins": origins, "profiles": profiles}
        data["hash"] = hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()
        return data

    def create(self, image, config, single, resume, password=""):
        paths = preflight_database(self.workspace, config["runtime"], self.offset, image)
        if config["server"].get("auth") and not password:
            raise ValueError("set admin_password in local.toml for the managed request launcher")
        settings.seed_templates(self.workspace)
        identifier = uuid.uuid4().hex
        directory = self.root / identifier
        directory.mkdir(parents=True, mode=0o700)
        # Queued jobs keep their launcher even after git pull changes the checkout.
        launcher = directory / "launcher"
        settings.snapshot_helper(launcher)
        for name in ("control", "worker"):
            shutil.copy2(settings.OPS_ROOT / f"slurm/{name}.sbatch", launcher / f"{name}.sbatch")
        for name in ("server", "runtime"):
            path = directory / f"{name}.toml"
            path.write_text(toml_dump(config[name]))
            path.chmod(0o600)
        write_json(directory / "launcher-auth.json", {"password": password})
        write_json(paths["marker"], {"model_version": image["build_info"]["model_version"]})
        manifest = {
            "version": 1,
            "id": identifier,
            "workspace": str(self.workspace),
            "port_offset": self.offset,
            "single_node": single,
            "resume_workers": resume,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "image": image,
            "config_hash": config["hash"],
            "origins": config["origins"],
            "profiles": config["profiles"],
            "database_paths": {k: str(v) for k, v in paths.items()},
            "server_config": str(directory / "server.toml"),
            "runtime_config": str(directory / "runtime.toml"),
            "server_digest": digest(directory / "server.toml"),
            "runtime_digest": digest(directory / "runtime.toml"),
            "control_job_id": None,
            "control_submission": "prepared",
            "state": "starting",
        }
        self.save(manifest)
        write_json(self.pointer, {"id": identifier})
        return manifest


def tag(manifest, suffix):
    return f"gb:{manifest['id']}:{suffix}"


def job_name(manifest, suffix):
    return f"gb-{manifest['id']}-{suffix}"


def owns(job, manifest, suffix=None):
    if suffix is None:
        return job.name.startswith(job_name(manifest, "")) and (
            not job.comment or job.comment.startswith(tag(manifest, ""))
        )
    return job.name == job_name(manifest, suffix) and (
        not job.comment or job.comment == tag(manifest, suffix)
    )
