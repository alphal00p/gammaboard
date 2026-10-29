"""Immutable submitted build recipes; publication and selection are separate."""

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import uuid

from .deployment import Deployments, cluster_config, digest, write_json
from .slurm import options, run


def resolve_revision(repository, revision):
    if re.fullmatch("[0-9a-fA-F]{40}", revision):
        return revision.lower()
    refs = run(
        [
            "git",
            "ls-remote",
            repository,
            revision,
            f"refs/heads/{revision}",
            f"refs/tags/{revision}",
            f"refs/tags/{revision}^{{}}",
        ]
    ).stdout.splitlines()
    peeled = [row.split()[0] for row in refs if row.split()[1].endswith("^{}")]
    hashes = set(peeled or [row.split()[0] for row in refs])
    if len(hashes) != 1:
        raise ValueError("revision is missing or ambiguous; specify a full commit SHA")
    return hashes.pop()


def prepare(workspace, family, revision="HEAD", output=None, definition=None):
    workspace = Path(workspace).resolve()
    repository = os.environ.get(
        "GAMMABOARD_REPO" if family == "gammaboard" else "GAMMALOOP_REPO",
        f"https://github.com/alphal00p/{family}.git",
    )
    sha = resolve_revision(repository, revision) if family != "apptainer" else None
    with Deployments(workspace, 0).locked():
        identifier = uuid.uuid4().hex
        build = workspace / "artifacts/builds" / identifier
        recipe = build / "recipe"
        build.mkdir(parents=True, mode=0o700)
        recipe.mkdir(mode=0o700)
        shutil.copy2(workspace / "ubelix.py", recipe / "ubelix.py")
        shutil.copytree(
            workspace / "gbops", recipe / "gbops", ignore=shutil.ignore_patterns("__pycache__")
        )
        shutil.copytree(
            workspace / "ops/build",
            recipe / "ops/build",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        record = {
            "id": identifier,
            "workspace": str(workspace),
            "family": family,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "repository": repository,
            "revision": sha,
            "requested_revision": revision,
            "state": "prepared",
            "job_id": None,
            "profile": cluster_config(workspace)["build"],
        }
        if family == "apptainer":
            source = (workspace / definition).resolve()
            target = (workspace / output).resolve()
            if not target.is_relative_to(workspace):
                raise ValueError("runtime output must be inside the workspace")
            if target.exists():
                raise ValueError("runtime output already exists")
            shutil.copy2(source, recipe / "runtime.def")
            record.update(output=str(target), definition_cwd=str(source.parent))
        write_json(
            recipe / "input.json", {k: v for k, v in record.items() if k not in ("state", "job_id")}
        )
        record["recipe_digest"] = recipe_digest(recipe)
        write_json(build / "build.json", record)
        return build / "build.json"


def recipe_digest(recipe):
    import hashlib

    h = hashlib.sha256()
    for path in sorted(recipe.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            h.update(str(path.relative_to(recipe)).encode() + b"\0" + bytes.fromhex(digest(path)))
    return h.hexdigest()


def submit(workspace, args, slurm):
    path = prepare(
        workspace,
        args.build_kind,
        getattr(args, "revision", "HEAD"),
        getattr(args, "output", None),
        getattr(args, "def_file", None),
    )
    with Deployments(workspace, 0).locked():
        record = json.loads(path.read_text())
        record.update(json.loads((path.parent / "recipe/input.json").read_text()))
        record["state"] = "submitting"
        write_json(path, record)
        env = dict(os.environ, GAMMABOARD_BUILD_RECORD=str(path))
        (Path(workspace) / "logs/slurm/build").mkdir(parents=True, exist_ok=True)
        record["job_id"] = slurm.submit(
            path.parent / "recipe/ops/build/build.sbatch",
            name=f"gb-build-{record['id']}",
            comment=f"gb:build:{record['id']}",
            workspace=workspace,
            env=env,
            options=options(record["profile"]),
        )
        record["state"] = "submitted"
        write_json(path, record)
    return record


def execute(path):
    path = Path(path).resolve()
    record = json.loads(path.read_text())
    build, recipe = path.parent, path.parent / "recipe"
    if recipe_digest(recipe) != record["recipe_digest"]:
        raise ValueError("submitted build recipe was modified")
    record.update(json.loads((recipe / "input.json").read_text()))
    env = dict(
        os.environ,
        GAMMABOARD_WORKSPACE_ROOT=record["workspace"],
        GAMMABOARD_BUILD_DIR=str(build),
        GAMMABOARD_RECIPE_ROOT=str(recipe),
        GAMMABOARD_SOURCE_REPOSITORY=record["repository"],
        GAMMABOARD_SOURCE_REVISION=record["revision"] or "",
    )
    record["state"] = "building"
    write_json(path, record)
    try:
        if record["family"] == "apptainer":
            target = Path(record["output"])
            target.parent.mkdir(parents=True, exist_ok=True)
            image = build / "image.sif"
            subprocess.run(
                ["apptainer", "build", str(image), str(recipe / "runtime.def")],
                cwd=record["definition_cwd"],
                env=env,
                check=True,
            )
            subprocess.run(["apptainer", "inspect", str(image)], env=env, check=True)
            os.link(image, target)
            target.chmod(0o444)
        else:
            subprocess.run(
                ["bash", str(recipe / "ops/build/service-build.sh"), record["family"]],
                env=env,
                check=True,
            )
        record["state"] = "completed"
    except BaseException:
        record["state"] = "failed"
        raise
    finally:
        write_json(path, record)


def select_image(workspace, path):
    workspace, path = Path(workspace).resolve(), Path(path).resolve()
    with Deployments(workspace, 0).locked():
        info = json.loads(path.read_text())
        family = info.get("family")
        if family not in ("gammaboard", "gammaloop"):
            raise ValueError("image must identify its service family")
        image = Path(info["path"]).resolve()
        if not image.is_relative_to(workspace / "images" / family):
            raise ValueError("select a published workspace image")
        if not info.get("validated") or digest(image) != info["sha256"]:
            raise ValueError("image validation failed")
        write_json(workspace / "images" / family / "current.json", info)
        return info
