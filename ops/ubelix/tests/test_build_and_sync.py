"""Exercise the actual build/sync scripts with isolated files and fake Slurm/container execution."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
sys.path.insert(0, str(ROOT))
from gbops import builds


def executable(path, contents):
    path.write_text(contents)
    path.chmod(0o755)


def command(args, **kw):
    return subprocess.run(
        args, check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw
    )


@pytest.fixture
def build_workspace(tmp_path, monkeypatch):
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    shutil.copy2(ROOT / "ubelix.py", workspace / "ubelix.py")
    shutil.copytree(ROOT / "config", workspace / "ops/config")
    shutil.copytree(ROOT / "build", workspace / "ops/build")
    shutil.copytree(
        ROOT / "gbops", workspace / "gbops", ignore=shutil.ignore_patterns("__pycache__")
    )
    repo = tmp_path / "repo"
    repo.mkdir()
    command(["git", "init", str(repo)])
    revisions = []
    for marker in ("a", "b"):
        (repo / "marker").write_text(marker)
        (repo / "Cargo.lock").write_text("# fixture lock\n")
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
        revisions.append(command(["git", "-C", str(repo), "rev-parse", "HEAD"]).stdout.strip())
    bindir = tmp_path / "bin"
    bindir.mkdir()
    (bindir / "python3").symlink_to(sys.executable)
    executable(
        bindir / "apptainer",
        """#!/usr/bin/env python3
import json,os,re,sys,time
from pathlib import Path
args=sys.argv[1:]
if args[0]=='build':
    image,definition=Path(args[-2]),Path(args[-1])
    text=definition.read_text()
    assert '@REVISION@' not in text and '@RUST_IMAGE@' not in text
    assert 'CARGO_TARGET_DIR=/build/target' in text
    source=Path(json.loads(re.search(r'%files\\n    (".*") /src',text).group(1)))
    barrier=os.environ.get('BUILD_BARRIER')
    if barrier:
        Path(barrier,image.parent.name).touch()
        end=time.monotonic()+10
        while len(list(Path(barrier).iterdir()))<2 and time.monotonic()<end:time.sleep(.02)
        assert len(list(Path(barrier).iterdir()))==2
    revision=re.search(r'GAMMABOARD_GIT_REVISION=([a-f0-9]+)',text).group(1)
    image.write_text(json.dumps({'revision':revision,'marker':(source/'marker').read_text(),'source':str(source)}))
elif args[0]=='exec':
    image=next(Path(arg) for arg in args if arg.endswith('.sif'))
    if args[-1]=='build-info':
        data=json.loads(image.read_text());print(json.dumps({'git_revision':data['revision'],'build_id':data['revision'],'model_version':7}))
    elif args[-1]=='/smoke/smoke.sh':
        if os.environ.get('FAIL_SMOKE'):sys.exit(1)
        (image.parent/'smoke.result').write_text('passed\\n')
    else:raise AssertionError(args)
else:raise AssertionError(args)
""",
    )
    env = dict(
        os.environ,
        PATH=f"{bindir}:{os.environ['PATH']}",
        GAMMABOARD_WORKSPACE_ROOT=str(workspace),
        GAMMABOARD_REPO=str(repo),
        PYTHONDONTWRITEBYTECODE="1",
    )
    monkeypatch.setenv("GAMMABOARD_REPO", str(repo))
    return workspace, revisions, env


def test_concurrent_builds_publish_only_their_own_sources_and_manifests(build_workspace, tmp_path):
    workspace, revisions, env = build_workspace
    barrier = tmp_path / "barrier"
    barrier.mkdir()
    records = [builds.prepare(workspace, "gammaboard", revision) for revision in revisions]
    # A later managed sync cannot change either already prepared recipe.
    (workspace / "ops/build/service-build.sh").write_text("exit 99\n")
    processes = [
        subprocess.Popen(
            [
                sys.executable,
                str(record.parent / "recipe/ubelix.py"),
                "run-build",
                "--record",
                str(record),
            ],
            env=dict(env, BUILD_BARRIER=str(barrier)),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        for record in records
    ]
    for process in processes:
        out, err = process.communicate(timeout=30)
        assert process.returncode == 0, (out, err)
    manifests = list((workspace / "images/gammaboard").glob("*/*/image.json"))
    assert len(manifests) == 2
    sources = set()
    for path in manifests:
        manifest = json.loads(path.read_text())
        image = Path(manifest["path"])
        payload = json.loads(image.read_text())
        index = revisions.index(manifest["source_revision"])
        assert payload["marker"] == ("a", "b")[index]
        assert manifest["sha256"] == hashlib.sha256(image.read_bytes()).hexdigest()
        assert manifest["build_info"]["git_revision"] == manifest["source_revision"]
        sources.add(payload["source"])
    assert len(sources) == 2
    assert not (workspace / "images/gammaboard/current.json").exists()
    chosen = builds.select_image(workspace, manifests[0])
    assert chosen["source_revision"] in revisions
    assert Path(chosen["path"]).is_file()


def test_failed_smoke_never_replaces_a_published_image(build_workspace):
    workspace, revisions, env = build_workspace
    first = builds.prepare(workspace, "gammaboard", revisions[0])
    command(
        [
            sys.executable,
            str(first.parent / "recipe/ubelix.py"),
            "run-build",
            "--record",
            str(first),
        ],
        env=env,
    )
    builds.select_image(workspace, next((workspace / "images/gammaboard").glob("*/*/image.json")))
    pointer = workspace / "images/gammaboard/current.json"
    before = pointer.read_bytes()
    second = builds.prepare(workspace, "gammaboard", revisions[1])
    result = subprocess.run(
        [
            sys.executable,
            str(second.parent / "recipe/ubelix.py"),
            "run-build",
            "--record",
            str(second),
        ],
        env=dict(env, FAIL_SMOKE="1"),
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0
    assert pointer.read_bytes() == before
    assert len(list((workspace / "images/gammaboard").glob("*/*/image.json"))) == 1


def test_sync_removes_deleted_managed_files_and_preserves_private_runtime_data(tmp_path):
    candidates = list(Path("/nix/store").glob("*rsync*/bin/rsync"))
    actual = shutil.which("rsync") or (str(candidates[0]) if candidates else None)
    if actual is None:
        pytest.skip("requires rsync")
    remote = tmp_path / "remote"
    remote.mkdir()
    bindir = tmp_path / "bin"
    bindir.mkdir()
    (bindir / "python3").symlink_to(sys.executable)
    executable(
        bindir / "ssh",
        """#!/usr/bin/env python3
import os,subprocess,sys
cmd=sys.argv[-1].replace('/storage/research/itp_localunitaritydata/user/workspace',os.environ['FAKE_REMOTE'])
sys.exit(subprocess.run(['bash','-c',cmd]).returncode)
""",
    )
    executable(
        bindir / "rsync",
        """#!/usr/bin/env python3
import os,sys
args=sys.argv[1:]
args[-1]=args[-1].replace('fake:/storage/research/itp_localunitaritydata/user/workspace',os.environ['FAKE_REMOTE'])
os.execv(os.environ['REAL_RSYNC'],[os.environ['REAL_RSYNC'],*args])
""",
    )
    for stale in (
        "ops/old.sh",
        "docs/upgrading-0.3.md",
        "gbops/old.py",
        "resources/templates/runs/old.toml",
    ):
        path = remote / stale
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("obsolete")
    protected = (
        "config/server.toml",
        "deployments/state.json",
        "images/active.sif",
        "resources/db/data",
        "resources/states/physics",
        "resources/runtimes/python",
    )
    for name in protected:
        path = remote / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("preserve")
    env = dict(
        os.environ,
        PATH=f"{bindir}:{os.environ['PATH']}",
        FAKE_REMOTE=str(remote),
        REAL_RSYNC=actual,
    )
    command(["bash", str(ROOT / "sync_ops.sh"), "fake", "user/workspace"], env=env)
    for stale in (
        "ops/old.sh",
        "docs/upgrading-0.3.md",
        "gbops/old.py",
        "resources/templates/runs/old.toml",
    ):
        assert not (remote / stale).exists()
    for name in protected:
        assert (remote / name).read_text() == "preserve"
    assert (remote / "resources/templates/nodes/cpu-workers.toml").exists()
    assert (remote / "docs/execution-model.md").exists()
    assert (remote / "gbops/slurm.py").exists()


def test_build_revision_is_resolved_before_queueing(build_workspace):
    workspace, revisions, env = build_workspace
    record = builds.prepare(workspace, "gammaboard", "HEAD")
    repository = Path(env["GAMMABOARD_REPO"])
    (repository / "marker").write_text("newer")
    command(["git", "-C", str(repository), "add", "."])
    command(
        [
            "git",
            "-C",
            str(repository),
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@invalid",
            "commit",
            "-m",
            "newer",
        ]
    )
    command(
        [
            sys.executable,
            str(record.parent / "recipe/ubelix.py"),
            "run-build",
            "--record",
            str(record),
        ],
        env=env,
    )
    published = json.loads((record.parent / "published.json").read_text())
    assert published["source_revision"] == revisions[-1]
    assert json.loads(Path(published["path"]).read_text())["marker"] == "b"


def test_successful_later_build_does_not_change_selection(build_workspace):
    workspace, revisions, env = build_workspace
    first = builds.prepare(workspace, "gammaboard", revisions[1])
    command(
        [
            sys.executable,
            str(first.parent / "recipe/ubelix.py"),
            "run-build",
            "--record",
            str(first),
        ],
        env=env,
    )
    builds.select_image(workspace, first.parent / "published.json")
    pointer = workspace / "images/gammaboard/current.json"
    selected = pointer.read_bytes()
    older = builds.prepare(workspace, "gammaboard", revisions[0])
    command(
        [
            sys.executable,
            str(older.parent / "recipe/ubelix.py"),
            "run-build",
            "--record",
            str(older),
        ],
        env=env,
    )
    assert pointer.read_bytes() == selected


def test_modified_build_recipe_is_rejected_before_execution(build_workspace):
    workspace, revisions, _env = build_workspace
    record = builds.prepare(workspace, "gammaboard", revisions[0])
    script = record.parent / "recipe/ops/build/service-build.sh"
    script.write_text("exit 0\n")
    with pytest.raises(ValueError, match="recipe was modified"):
        builds.execute(record)
