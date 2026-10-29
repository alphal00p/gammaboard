"""Stop checks and explicit database reset."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import tomllib
import uuid

from . import settings
from .deployment import database_paths, owns, write_json, digest


def require_idle(manifest, slurm):
    if manifest is None:
        return
    if any(owns(job, manifest) for job in slurm.active()):
        raise RuntimeError("deployment still has Slurm jobs; run down first")
    if manifest["state"] == "stopped":
        return
    if manifest.get("control_submission") == "prepared":
        return
    job = (
        slurm.lookup(manifest["control_job_id"], manifest["created_at"][:10])
        if manifest.get("control_job_id")
        else None
    )
    if job is None or not owns(job, manifest, "control") or not job.terminal:
        raise RuntimeError(
            "deployment shutdown is not confirmed; run down or resolve-control first"
        )


def reset_database(deployments, slurm, discard=False):
    # Caller holds the workspace lock, excluding startup and new submissions.
    manifest = deployments.load()
    require_idle(manifest, slurm)
    if manifest:
        runtime_path = Path(manifest["runtime_config"])
        if digest(runtime_path) != manifest["runtime_digest"]:
            raise ValueError("deployment runtime snapshot was modified; refusing database reset")
        runtime = tomllib.loads(runtime_path.read_text())
    else:
        runtime = settings.local_config().get("runtime", {})
    paths = database_paths(deployments.workspace, runtime, deployments.offset)
    active = slurm.active()
    for path in deployments.root.glob("*/manifest.json"):
        other = json.loads(path.read_text())
        if other["workspace"] != str(deployments.workspace) or not any(
            owns(job, other) for job in active
        ):
            continue
        other_runtime = tomllib.loads(Path(other["runtime_config"]).read_text())
        other_paths = database_paths(deployments.workspace, other_runtime, other["port_offset"])
        if other["port_offset"] == deployments.offset or any(
            a.resolve().is_relative_to(b.resolve()) or b.resolve().is_relative_to(a.resolve())
            for a in paths.values()
            for b in other_paths.values()
        ):
            raise RuntimeError("another deployment using these database paths still has Slurm jobs")
    if manifest is None and paths["data_dir"].exists() and any(paths["data_dir"].iterdir()):
        raise RuntimeError(
            "untracked database: stop the legacy deployment and maintain its data manually"
        )
    root = deployments.workspace / "resources/db"
    if root.resolve() != root:
        raise ValueError("database root must not contain symlinks")
    selected = []
    for path in paths.values():
        # Never follow a private override or symlink into physics/config/image trees.
        if (
            path.is_symlink()
            or not path.resolve().is_relative_to(root.resolve())
            or path.resolve() == root.resolve()
        ):
            raise ValueError(f"database reset requires a path strictly inside {root}: {path}")
        if path.exists():
            selected.append(path)
    selected = list(dict.fromkeys(selected))
    selected = [p for p in selected if not any(p != q and p.is_relative_to(q) for q in selected)]
    archive = (
        deployments.workspace
        / "artifacts/database-archives"
        / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8])
    )
    if selected and not discard:
        archive.mkdir(parents=True, mode=0o700)
        write_json(
            archive / "archive.json",
            {
                "deployment": manifest,
                "paths": [
                    {"source": str(path), "archived": f"{i}-{path.name}"}
                    for i, path in enumerate(selected)
                ],
            },
        )
        if manifest:
            for name in ("server", "runtime"):
                shutil.copy2(manifest[name + "_config"], archive / f"{name}.toml")
    for i, path in enumerate(selected):
        if discard:
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()
        else:
            path.rename(archive / f"{i}-{path.name}")
    return {
        "discarded": discard,
        "paths": [str(p) for p in selected],
        "archive": str(archive) if selected and not discard else None,
    }


def require_stopped(workspace, slurm):
    """Replacing any service/runtime image requires stopping the whole workspace."""
    root = Path(workspace) / "deployments"
    manifests = [json.loads(path.read_text()) for path in root.glob("*/manifest.json")]
    if any(owns(job, manifest) for job in slurm.active() for manifest in manifests):
        raise RuntimeError("workspace still has Slurm jobs; run down for each deployment first")
    current = {json.loads(path.read_text())["id"] for path in root.glob("active-*.json")}
    for manifest in manifests:
        if manifest["id"] in current:
            require_idle(manifest, slurm)
