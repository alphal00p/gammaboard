"""Exercise checkout-based builds using real Git and fake Slurm/Apptainer."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

from test_ubelix import ROOT, FakeSlurm, configure, create_manifest, control
import ubelix as u
from gbops import builds, settings
from gbops.deployment import Deployments, toml_dump
from gbops.slurm import Job, SchedulerError


def executable(path, contents):
    path.write_text(contents)
    path.chmod(0o755)


def command(args, **kw):
    return subprocess.run(
        args, check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw
    )


def commit(repo, marker):
    (repo / "marker").write_text(marker)
    command(["git", "-C", str(repo), "add", "."])
    command(
        [
            "git",
            "-C",
            str(repo),
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@invalid",
            "commit",
            "-m",
            marker,
        ]
    )
    return command(["git", "-C", str(repo), "rev-parse", "HEAD"]).stdout.strip()


@pytest.fixture
def checkout(tmp_path, monkeypatch):
    repo = tmp_path / "checkout"
    ops = repo / "ops/ubelix"
    shutil.copytree(ROOT, ops, ignore=shutil.ignore_patterns("__pycache__", "local.toml"))
    shutil.copytree(ROOT.parents[1] / "resources/templates", repo / "resources/templates")
    (repo / ".gitignore").write_text("ops/ubelix/local.toml\n__pycache__/\n")
    command(["git", "init", str(repo)])
    commit(repo, "a")
    workspace = tmp_path / "state"
    (ops / "local.toml").write_text(toml_dump({"workspace_root": str(workspace)}))
    monkeypatch.setattr(settings, "OPS_ROOT", ops)
    bindir = tmp_path / "bin"
    bindir.mkdir()
    (bindir / "python3").symlink_to(sys.executable)
    for scheduler in ("squeue", "sacct"):
        executable(bindir / scheduler, "#!/usr/bin/env bash\nexit 0\n")
    executable(
        bindir / "apptainer",
        r"""#!/usr/bin/env python3
import json,os,re,sys
from pathlib import Path
args=sys.argv[1:]
if args[0]=='build':
    image,definition=Path(args[-2]),Path(args[-1])
    text=definition.read_text()
    if definition.name=='runtime.def':
        image.write_text(text)
        sys.exit(0)
    assert '@REVISION@' not in text and '@RUST_IMAGE@' not in text
    assert 'CARGO_TARGET_DIR=/build/target' in text
    source=Path(json.loads(re.search(r'%files\n    (".*") /src',text).group(1)))
    assert not (source/'ops/ubelix/local.toml').exists()
    revision=re.search(r'GAMMABOARD_GIT_REVISION=([a-f0-9]+)',text).group(1)
    image.write_text(json.dumps({'revision':revision,'marker':(source/'marker').read_text()}))
elif args[0]=='exec':
    image=next(Path(arg) for arg in args if arg.endswith('.sif'))
    if args[-1]=='build-info':
        data=json.loads(image.read_text());print(json.dumps({'git_revision':data['revision'],'build_id':data['revision'],'model_version':7}))
    elif args[-1]=='/smoke/smoke.sh':
        if os.environ.get('FAIL_SMOKE'):sys.exit(1)
        (image.parent/'smoke.result').write_text('passed\n')
    elif args[-2:]==['gammaloop','--help']:print('GammaLoop')
    else:raise AssertionError(args)
elif args[0]=='inspect':assert Path(args[1]).is_file()
else:raise AssertionError(args)
""",
    )
    env = dict(os.environ, PATH=f"{bindir}:{os.environ['PATH']}", PYTHONDONTWRITEBYTECODE="1")
    return repo, workspace, env


def submit(workspace, scheduler, family="gammaboard", *args):
    return builds.submit(workspace, u.parser().parse_args(["build", family, *args]), scheduler)


def execute(workspace, record, env, scheduler, **extra):
    path = workspace / "artifacts/builds" / record["id"] / "build.json"
    result = subprocess.run(
        [sys.executable, str(path.parent / "recipe/ubelix.py"), "run-build", "--record", str(path)],
        env=dict(env, **extra),
        capture_output=True,
        text=True,
    )
    old = scheduler.jobs[record["job_id"]]
    scheduler.jobs[old.id] = Job(
        old.id, old.name, "COMPLETED" if result.returncode == 0 else "FAILED", "", old.comment
    )
    return result, json.loads(path.read_text())


def test_renderer_rejects_tag_and_digest_before_apptainer(checkout, tmp_path):
    _, _, env = checkout
    lock = settings.OPS_ROOT / "build/images.lock.json"
    pins = json.loads(lock.read_text())
    pins["node"]["image"] = pins["node"]["image"].replace("@", ":20-bookworm-slim@")
    lock.write_text(json.dumps(pins))
    rendered = tmp_path / "rendered"
    rendered.mkdir()
    result = subprocess.run(
        [
            sys.executable,
            str(settings.OPS_ROOT / "build/render.py"),
            str(settings.OPS_ROOT),
            str(rendered),
            "gammaboard",
            "a" * 40,
            "4",
        ],
        env=env,
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0 and "without a tag" in result.stderr
    assert not (rendered / "service.def").exists()


def test_build_replaces_fixed_image_and_failed_smoke_preserves_it(checkout):
    repo, workspace, env = checkout
    scheduler = FakeSlurm()
    first = submit(workspace, scheduler)
    result, saved = execute(workspace, first, env, scheduler)
    assert result.returncode == 0, result.stderr
    image = workspace / "images/gammaboard.sif"
    metadata = image.with_suffix(".json")
    assert saved["state"] == "completed" and saved["image"] == str(image)
    assert json.loads(image.read_text())["marker"] == "a"
    before = image.read_bytes(), metadata.read_bytes()
    revision = commit(repo, "b")
    result, saved = execute(workspace, submit(workspace, scheduler), env, scheduler, FAIL_SMOKE="1")
    assert result.returncode != 0 and saved["state"] == "failed"
    assert (image.read_bytes(), metadata.read_bytes()) == before
    result, saved = execute(workspace, submit(workspace, scheduler), env, scheduler)
    assert result.returncode == 0, result.stderr
    info = json.loads(metadata.read_text())
    assert info["source_revision"] == revision
    assert info["sha256"] == hashlib.sha256(image.read_bytes()).hexdigest()
    assert json.loads(image.read_text())["marker"] == "b"
    assert sorted(p.name for p in image.parent.iterdir()) == ["gammaboard.json", "gammaboard.sif"]
    assert not (Path(info["build_directory"]) / "source.tar").exists()
    assert not (Path(info["build_directory"]) / "src").exists()
    # The normal deployment automatically uses the replacement.
    assert Deployments(workspace, 0).image()["source_revision"] == revision


def test_queued_build_is_independent_of_checkout_and_local_settings(checkout):
    repo, workspace, env = checkout
    scheduler = FakeSlurm()
    record = submit(workspace, scheduler)
    commit(repo, "newer")
    (settings.OPS_ROOT / "build/service-build.sh").write_text("exit 99\n")
    shutil.rmtree(repo)
    result, saved = execute(workspace, record, env, scheduler)
    assert result.returncode == 0, result.stderr
    assert json.loads(Path(saved["image"]).read_text())["marker"] == "a"
    assert (
        json.loads(Path(saved["image"]).with_suffix(".json").read_text())["source_revision"]
        == record["revision"]
    )


@pytest.mark.parametrize("change", ["tracked", "staged", "untracked"])
def test_dirty_source_rejected_before_submission(checkout, change):
    repo, workspace, _ = checkout
    (repo / ("extra" if change == "untracked" else "marker")).write_text("dirty")
    if change == "staged":
        command(["git", "-C", str(repo), "add", "."])
    scheduler = FakeSlurm()
    with pytest.raises(ValueError, match="commit or discard"):
        submit(workspace, scheduler)
    assert not scheduler.submissions


def test_build_and_deployment_exclude_each_other_across_offsets(checkout, monkeypatch):
    _, workspace, env = checkout
    scheduler = FakeSlurm()
    record = submit(workspace, scheduler)
    for state in ("PENDING", "RUNNING"):
        old = scheduler.jobs[record["job_id"]]
        scheduler.jobs[old.id] = Job(old.id, old.name, state, "", old.comment)
        with pytest.raises(RuntimeError, match="queued/running build"):
            submit(workspace, scheduler)
        with pytest.raises(RuntimeError, match="queued/running build"):
            u.up(u.parser().parse_args(["up", "--port-offset", "10"]), scheduler)
    result, _ = execute(workspace, record, env, scheduler)
    assert result.returncode == 0, result.stderr
    manifest = create_manifest(workspace, 10)
    for state in ("PENDING", "RUNNING"):
        control(manifest, scheduler, state)
        with pytest.raises(RuntimeError, match="run down"):
            submit(workspace, scheduler)
    monkeypatch.setattr(u, "wait_ready", lambda *_: ("compute-a", None))
    u.down(u.parser().parse_args(["down", "--force", "--port-offset", "10"]), scheduler)
    assert submit(workspace, scheduler)["state"] == "submitted"


def test_build_scheduler_failure_does_not_submit(checkout):
    _, workspace, _ = checkout
    scheduler = FakeSlurm()
    scheduler.error = True
    with pytest.raises(SchedulerError):
        submit(workspace, scheduler)
    assert not scheduler.submissions


def test_gammaloop_uses_configured_local_checkout(checkout):
    repo, workspace, env = checkout
    source = repo.parent / "gammaloop"
    command(["git", "clone", str(repo), str(source)])
    revision = commit(source, "loop")
    configure(gammaloop_source="../../../gammaloop")
    scheduler = FakeSlurm()
    result, saved = execute(workspace, submit(workspace, scheduler, "gammaloop"), env, scheduler)
    assert result.returncode == 0, result.stderr
    image = workspace / "images/gammaloop.sif"
    assert saved["revision"] == revision and saved["image"] == str(image)
    assert json.loads(image.read_text())["marker"] == "loop"


def test_generic_runtime_uses_frozen_definition_and_never_overwrites(checkout):
    repo, workspace, env = checkout
    definition = repo.parent / "runtime.def"
    definition.write_text("original definition")
    scheduler = FakeSlurm()
    record = submit(
        workspace, scheduler, "apptainer", "resources/runtimes/custom.sif", str(definition)
    )
    definition.write_text("edited definition")
    result, saved = execute(workspace, record, env, scheduler)
    assert result.returncode == 0, result.stderr
    assert Path(saved["output"]).read_text() == "original definition"
    with pytest.raises(ValueError, match="already exists"):
        submit(workspace, scheduler, "apptainer", "resources/runtimes/custom.sif", str(definition))


def test_local_config_is_relative_to_helper_not_cwd_or_environment(checkout, monkeypatch, tmp_path):
    _, workspace, _ = checkout
    configure(workspace_root="../../../state", cluster={"control": {"mem_per_cpu": "8G"}})
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("GAMMABOARD_WORKSPACE_ROOT", str(tmp_path / "ignored"))
    assert settings.workspace_root() == workspace
    profile = Deployments(workspace, 0).configuration(False, 8080)["profiles"]["control"]
    assert profile["mem_per_cpu"] == "8G" and "mem" not in profile
    (settings.OPS_ROOT / "local.toml").unlink()
    with pytest.raises(ValueError, match="local.example.toml"):
        settings.workspace_root()
    assert u.parser().parse_args(["build", "gammaboard"]).build_kind == "gammaboard"
