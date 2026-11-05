"""Repository assets and machine-local settings; job snapshots are self-contained."""

from pathlib import Path
import shutil
import tomllib

OPS_ROOT = Path(__file__).resolve().parents[1]

# Reuse the canonical portable cards; cluster-specific additions live beside the
# UBELIX node cards. Generated physics states and external runtimes are optional.
PORTABLE_RUNS = (
    "installation-smoke.toml",
    "tetrahedron-surface.toml",
    "hyperparameter-tuning.toml",
    "parameter-scan.toml",
    "support/tetrahedron-trial.toml",
)


def local_config():
    path = OPS_ROOT / "local.toml"
    if not path.is_file():
        raise ValueError(f"create {path} from local.example.toml and set workspace_root")
    return tomllib.loads(path.read_text())


def local_path(value):
    return (OPS_ROOT / Path(value).expanduser()).resolve()


def workspace_root():
    value = local_config().get("workspace_root")
    if not isinstance(value, str) or not value.strip():
        raise ValueError("set workspace_root in local.toml")
    return local_path(value)


def snapshot_helper(target):
    target.mkdir(parents=True, mode=0o700)
    shutil.copy2(OPS_ROOT / "ubelix.py", target / "ubelix.py")
    shutil.copytree(
        OPS_ROOT / "gbops", target / "gbops", ignore=shutil.ignore_patterns("__pycache__")
    )


def seed_templates(workspace):
    target = Path(workspace) / "resources/templates"
    if not target.exists():
        shutil.copytree(OPS_ROOT.parents[1] / "resources/templates/tasks", target / "tasks")
        shutil.copytree(OPS_ROOT / "resources/templates/nodes", target / "nodes")
        install_examples(workspace)


def install_examples(workspace):
    """Refresh only the bundled portable run cards, including their child files."""
    target = Path(workspace) / "resources/templates/runs"
    common = OPS_ROOT.parents[1] / "resources/templates/runs"
    cluster = OPS_ROOT / "resources/templates/runs"
    sources = [(common / name, Path(name)) for name in PORTABLE_RUNS]
    sources += [(path, path.relative_to(cluster)) for path in sorted(cluster.rglob("*.toml"))]
    for source, name in sources:
        destination = target / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    return {"directory": str(target), "installed": [str(name) for _, name in sources]}
