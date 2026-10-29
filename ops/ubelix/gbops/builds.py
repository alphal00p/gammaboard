"""Build checked-out sources, validate them, and replace the workspace image."""

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import uuid

from . import settings
from .deployment import Deployments, cluster_config, digest, write_json
from .maintenance import require_stopped
from .slurm import options, run


def require_no_build(workspace, slurm):
    names = {f"gb-build-{path.name}" for path in (Path(workspace) / "artifacts/builds").glob("*")}
    if any(job.name in names for job in slurm.active()):
        raise RuntimeError(
            "workspace has a queued/running build; wait for it or cancel it with scancel"
        )


def clean_revision(repository):
    if run(["git", "-C", str(repository), "status", "--porcelain"]).stdout.strip():
        raise ValueError(f"commit or discard source changes before building: {repository}")
    return run(["git", "-C", str(repository), "rev-parse", "HEAD"]).stdout.strip()


def prepare(workspace, family, output=None, definition=None):
    """Called under the workspace lock, before submitting the batch job."""
    workspace = Path(workspace).resolve()
    source, revision = None, None
    if family != "apptainer":
        source = settings.OPS_ROOT.parents[1]
        revision = clean_revision(source)
        if family == "gammaloop":
            value = settings.local_config().get("gammaloop_source")
            if not value:
                raise ValueError("set gammaloop_source in local.toml to a local GammaLoop checkout")
            source = settings.local_path(value)
            revision = clean_revision(source)
    identifier = uuid.uuid4().hex
    build = workspace / "artifacts/builds" / identifier
    recipe = build / "recipe"
    settings.snapshot_helper(recipe)
    shutil.copytree(
        settings.OPS_ROOT / "build", recipe / "build", ignore=shutil.ignore_patterns("__pycache__")
    )
    record = {
        "id": identifier,
        "workspace": str(workspace),
        "family": family,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "source": str(source) if source else None,
        "revision": revision,
        "state": "prepared",
        "job_id": None,
        "profile": cluster_config()["build"],
    }
    if family == "apptainer":
        definition = Path(definition).resolve()
        target = (workspace / output).resolve()
        if not target.is_relative_to(workspace):
            raise ValueError("runtime output must be inside the workspace")
        if target.exists():
            raise ValueError("runtime output already exists; remove it while stopped to rebuild")
        shutil.copy2(definition, recipe / "runtime.def")
        record.update(output=str(target), definition_cwd=str(definition.parent))
    else:
        # Archive this local commit now. Queued jobs do not depend on a remote,
        # the working tree, or Git history remaining available in the checkout.
        run(
            [
                "git",
                "-C",
                str(source),
                "archive",
                "--format=tar",
                f"--output={build / 'source.tar'}",
                revision,
            ],
            timeout=120,
        )
    write_json(build / "build.json", record)
    return build / "build.json"


def submit(workspace, args, slurm):
    with Deployments(workspace, 0).locked():
        require_stopped(workspace, slurm)
        require_no_build(workspace, slurm)
        path = prepare(
            workspace,
            args.build_kind,
            getattr(args, "output", None),
            getattr(args, "def_file", None),
        )
        record = json.loads(path.read_text())
        record["state"] = "submitting"
        write_json(path, record)
        env = dict(os.environ, GAMMABOARD_BUILD_RECORD=str(path))
        (Path(workspace) / "logs/slurm/build").mkdir(parents=True, exist_ok=True)
        record["job_id"] = slurm.submit(
            path.parent / "recipe/build/build.sbatch",
            name=f"gb-build-{record['id']}",
            comment=f"gb:build:{record['id']}",
            workspace=workspace,
            env=env,
            options=options(record["profile"]),
        )
        record["state"] = "submitted"
        write_json(path, record)
    return record


def publish(build, record):
    if (build / "smoke.result").read_text().strip() != "passed":
        raise ValueError("image smoke validation did not pass")
    info = json.loads((build / "build-info.json").read_text())
    if record["family"] == "gammaboard" and info["git_revision"] != record["revision"]:
        raise ValueError("built executable provenance differs from the checked-out commit")
    image = build / "image.sif"
    target = Path(record["workspace"]) / "images" / (record["family"] + ".sif")
    metadata = {
        "path": str(target),
        "sha256": digest(image),
        "source_revision": record["revision"],
        "family": record["family"],
        "build_info": info,
        "base_images": json.loads((build / "images.lock.json").read_text()),
        "build_directory": str(build),
        "validated": True,
    }
    target.parent.mkdir(parents=True, exist_ok=True)
    image.chmod(0o444)
    os.replace(image, target)
    write_json(target.with_suffix(".json"), metadata)
    return str(target)


def execute(path, slurm):
    path = Path(path).resolve()
    build, recipe = path.parent, path.parent / "recipe"
    record = json.loads(path.read_text())
    deployments = Deployments(record["workspace"], 0)
    # The submitter saves its receipt before this job reads or updates the record.
    with deployments.locked():
        record = json.loads(path.read_text())
        require_stopped(record["workspace"], slurm)
        record["state"] = "building"
        write_json(path, record)
    env = dict(
        os.environ,
        GAMMABOARD_WORKSPACE_ROOT=record["workspace"],
        GAMMABOARD_BUILD_DIR=str(build),
        GAMMABOARD_RECIPE_ROOT=str(recipe),
        GAMMABOARD_SOURCE_REVISION=record["revision"] or "",
    )
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
            with deployments.locked():
                require_stopped(record["workspace"], slurm)
                os.link(image, target)
                target.chmod(0o444)
        else:
            with tarfile.open(build / "source.tar") as archive:
                archive.extractall(build / "src", filter="data")
            subprocess.run(
                ["bash", str(recipe / "build/service-build.sh"), record["family"]],
                env=env,
                check=True,
            )
            with deployments.locked():
                require_stopped(record["workspace"], slurm)
                record["image"] = publish(build, record)
            # Successful builds retain logs and provenance, not redundant source/targets.
            for name in ("src", "tmp"):
                shutil.rmtree(build / name, ignore_errors=True)
            (build / "source.tar").unlink()
        record["state"] = "completed"
    except BaseException:
        record["state"] = "failed"
        raise
    finally:
        write_json(path, record)
