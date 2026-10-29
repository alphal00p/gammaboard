"""Explicit, scoped data reset and reference-aware artifact pruning."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import tomllib
import uuid

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
    runtime_path = (
        Path(manifest["runtime_config"])
        if manifest
        else deployments.workspace / "config/runtime.toml"
    )
    if manifest and digest(runtime_path) != manifest["runtime_digest"]:
        raise ValueError("deployment runtime snapshot was modified; refusing database reset")
    runtime = tomllib.loads(runtime_path.read_text()) if runtime_path.exists() else {}
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


def prune(deployments, slurm, apply=False, keep=1):
    workspace = deployments.workspace
    active = slurm.active()
    retained, retired = [], []
    pointers = {json.loads(p.read_text())["id"] for p in deployments.root.glob("active-*.json")}
    for path in deployments.root.glob("*/manifest.json"):
        manifest = json.loads(path.read_text())
        if Path(manifest["workspace"]) != workspace:
            raise ValueError("foreign workspace manifest")
        live = any(owns(j, manifest) for j in active)
        if manifest["id"] in pointers or live:
            retained.append(manifest)
        else:
            try:
                require_idle(manifest, slurm)
            except RuntimeError:
                retained.append(manifest)
            else:
                retired.append(manifest)
    retired.sort(key=lambda m: m["created_at"], reverse=True)
    by_offset = {}
    candidates = []
    for manifest in retired:
        offset = manifest["port_offset"]
        if by_offset.get(offset, 0) < keep:
            retained.append(manifest)
            by_offset[offset] = by_offset.get(offset, 0) + 1
        else:
            candidates.append(deployments.root / manifest["id"])
    referenced = {Path(m["image"]["path"]).resolve() for m in retained}
    for path in (workspace / "artifacts/database-archives").glob("*/archive.json"):
        archived = json.loads(path.read_text()).get("deployment")
        if archived:
            referenced.add(Path(archived["image"]["path"]).resolve())
    for pointer in (workspace / "images").glob("*/current.json"):
        referenced.add(Path(json.loads(pointer.read_text())["path"]).resolve())
    for path in (workspace / "images").glob("*/*/*/image.json"):
        image = Path(json.loads(path.read_text())["path"]).resolve()
        if image.parent != path.parent.resolve():
            raise ValueError("published image path differs from its metadata directory")
        if image not in referenced:
            candidates.append(path.parent)
    for path in (workspace / "artifacts/builds").glob("*/build.json"):
        record = json.loads(path.read_text())
        identifier = record.get("job_id")
        if identifier:
            job = slurm.lookup(identifier, record["created_at"][:10])
            if job is None or not job.terminal or job.name != f"gb-build-{record['id']}":
                continue
        else:
            continue
        candidates.append(path.parent)
    roots = [deployments.root, workspace / "images", workspace / "artifacts/builds"]
    if any(root.resolve() != root for root in roots):
        raise ValueError("artifact roots must not contain symlinks")
    for path in candidates:
        if path.is_symlink() or not any(
            path.resolve().is_relative_to(root.resolve()) and path.resolve() != root.resolve()
            for root in roots
        ):
            raise ValueError(f"unsafe artifact path: {path}")
    if apply:
        for path in candidates:
            shutil.rmtree(path)
    return {
        "applied": apply,
        "paths": [str(p) for p in candidates],
        "preserved": [
            "current deployments/images",
            "referenced images",
            f"{keep} retired deployments per offset",
            "unknown or active builds",
            "private configuration",
            "physics states",
            "process runtimes",
            "database archives",
            "logs",
        ],
    }
