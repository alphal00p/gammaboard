"""Explicit helper and instance configuration; frozen jobs do not read mutable config."""
from pathlib import Path
import copy
import shutil
import tomllib

OPS_ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = None
PORTABLE_RUNS = (
    "installation-smoke.toml", "tetrahedron-native.toml", "tetrahedron-tuning.toml",
    "tetrahedron-scan.toml", "tetrahedron-campaign.toml", "support/tetrahedron-trial.toml",
)

def configure(path):
    global CONFIG_PATH
    CONFIG_PATH = Path(path).expanduser().resolve() if path else None

def config_path():
    # Legacy local.toml is accepted for compatibility; new usage passes --config.
    return CONFIG_PATH or OPS_ROOT / "local.toml"

def local_config():
    path = config_path()
    if not path.is_file():
        raise ValueError(f"helper configuration not found: {path}; pass --config FILE (see examples/)")
    return tomllib.loads(path.read_text())

def local_path(value):
    return (config_path().parent / Path(value).expanduser()).resolve()

def workspace_root():
    value = local_config().get("workspace_root")
    if not isinstance(value,str) or not value.strip():
        raise ValueError("set explicit workspace_root in the helper configuration")
    return local_path(value)

def instance_definition():
    value = local_config().get("instance_file")
    if value is None:
        return None
    path = local_path(value)
    data = copy.deepcopy(tomllib.loads(path.read_text()))
    if data.get("externally_managed") is not True:
        raise ValueError("the HPC instance must set externally_managed = true")
    data.pop("execution", None)
    for key in ("state_dir", "log_dir", "temp_dir", "template_dir"):
        data["paths"][key] = str((path.parent / data["paths"][key]).resolve())
    data["paths"]["resource_roots"] = [str((path.parent / root).resolve()) for root in data["paths"].get("resource_roots", [])]
    database = data["database"]
    if database.get("mode") != "managed":
        raise ValueError("the Slurm control helper currently requires a managed database")
    for key in ("data_dir", "socket_dir"):
        database[key] = str((path.parent / database[key]).resolve())
    server = data.setdefault("server", {})
    dashboard = server.get("dashboard_dir", "/usr/local/share/gammaboard/dashboard")
    server["dashboard_dir"] = str((path.parent / dashboard).resolve())
    if "job_env_file" in data:
        raise ValueError("job_env_file belongs to the helper configuration")
    return data

def snapshot_environment(target):
    value = local_config().get("job_env_file")
    if value is None:
        return None
    source = local_path(value)
    if not source.is_file():
        raise ValueError(f"configured job_env_file does not exist: {source}")
    target = Path(target)
    shutil.copy2(source, target)
    target.chmod(0o600)
    return str(target)

def snapshot_helper(target):
    target.mkdir(parents=True, mode=0o700)
    shutil.copy2(OPS_ROOT / "slurm.py", target / "slurm.py")
    shutil.copytree(OPS_ROOT / "gbops", target / "gbops", ignore=shutil.ignore_patterns("__pycache__"))

def template_root(workspace):
    instance = instance_definition()
    return Path(instance["paths"]["template_dir"]) if instance else Path(workspace)/"resources/templates"

def seed_templates(workspace):
    target = template_root(workspace)
    if not target.exists():
        shutil.copytree(OPS_ROOT.parents[1] / "resources/templates/tasks", target / "tasks")
        shutil.copytree(OPS_ROOT / "resources/templates/nodes", target / "nodes")
        install_examples(workspace)

def install_examples(workspace):
    target = template_root(workspace)/"runs"
    common = OPS_ROOT.parents[1] / "resources/templates/runs"
    for name in PORTABLE_RUNS:
        destination = target/name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(common/name,destination)
    return {"directory":str(target),"installed":list(PORTABLE_RUNS)}
