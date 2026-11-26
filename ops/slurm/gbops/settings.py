"""Explicit helper and instance configuration; frozen jobs do not read mutable config."""
from pathlib import Path
import copy
import os
import pwd
import tempfile
import shutil
import tomllib

OPS_ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = None
PORTABLE_RUNS = (
    "installation-smoke.toml", "tetrahedron-native.toml", "tetrahedron-unstratified.toml", "tetrahedron-tuning.toml",
    "tetrahedron-scan.toml", "tetrahedron-campaign.toml", "support/tetrahedron-trial.toml",
)

def configure(path):
    global CONFIG_PATH
    CONFIG_PATH = Path(path).expanduser().resolve() if path else None

def default_workspace():
    checkout = OPS_ROOT.parents[1]
    return checkout.with_name(checkout.name + "-workspace")

def config_path():
    return CONFIG_PATH or default_workspace() / "slurm.toml"

def login_user():
    return pwd.getpwuid(os.getuid()).pw_name

def ssh_target():
    config = local_config()
    access = config.get("access", {})
    explicit = access.get("ssh_target") or config.get("ssh_target")
    if explicit:
        return explicit
    host = access.get("ssh_host")
    return f"{login_user()}@{host}" if host else None

def initialize(workspace=None, site="ubelix", username=None, target=None):
    from .deployment import toml_dump
    config = CONFIG_PATH or ((Path(workspace).expanduser().resolve() if workspace else default_workspace()) / "slurm.toml")
    root = Path(workspace).expanduser().resolve() if workspace else config.parent
    checkout = OPS_ROOT.parents[1].resolve()
    for path in (root, config.parent):
        if path.resolve().is_relative_to(checkout):
            raise ValueError("keep the workspace and generated configuration outside the checkout")
    paths = [config, config.parent / "gammaboard.toml", config.parent / "environment.sh"]
    if len(set(paths)) != len(paths):
        raise ValueError("helper configuration filename must differ from gammaboard.toml and environment.sh")
    for path in paths:
        if path.exists() or path.is_symlink():
            raise ValueError(f"refusing to overwrite existing configuration: {path}; select it with --config or edit it in place")
    helper = tomllib.loads((OPS_ROOT / "examples" / f"{site}.toml").read_text())
    instance = tomllib.loads((OPS_ROOT / "examples/gammaboard.toml").read_text())
    relative = Path(os.path.relpath(root, config.parent))
    helper.update(workspace_root=str(relative), image=str(relative / "images/gammaboard.sif"), binds=[str(relative)])
    access = helper["access"]
    access["ssh_target"] = target or f"{username or login_user()}@{access.pop('ssh_host')}"
    access.pop("ssh_host", None)
    for key in ("state_dir", "log_dir", "temp_dir", "template_dir"):
        instance["paths"][key] = str(relative / instance["paths"][key])
    instance["paths"]["resource_roots"] = [str(relative / path) for path in instance["paths"]["resource_roots"]]
    for key in ("data_dir", "socket_dir"):
        instance["database"][key] = str(relative / instance["database"][key])
    contents = [toml_dump(helper), toml_dump(instance), (OPS_ROOT / "examples/environment.sh").read_text()]
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    config.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    created = []
    with tempfile.TemporaryDirectory(dir=config.parent, prefix=".init-") as staging:
        try:
            for index, (path, content) in enumerate(zip(paths, contents)):
                staged = Path(staging) / str(index)
                staged.write_text(content)
                staged.chmod(0o600)
                os.link(staged, path) # Atomic exclusive creation, including against concurrent init.
                created.append(path)
        except BaseException:
            for path in created:
                path.unlink()
            raise
    configure(config)
    return {"config": str(config), "workspace": str(root), "ssh_target": access["ssh_target"]}


def local_config():
    path = config_path()
    if not path.is_file():
        raise ValueError(f"helper configuration not found: {path}; run python3 ops/slurm/slurm.py init, or select --config FILE / GAMMABOARD_SLURM_CONFIG")
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
    if "execution" in data:
        raise ValueError("remove obsolete execution settings; use externally_managed = true")
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
    if not (target / "tasks").exists():
        shutil.copytree(OPS_ROOT.parents[1] / "resources/templates/tasks", target / "tasks")
    if not (target / "nodes").exists():
        (target / "nodes").mkdir(parents=True)
        for name in ("cpu-workers.toml", "free-gpu-plus-cpu.toml"):
            shutil.copy2(OPS_ROOT / "examples" / name, target / "nodes" / name)
    if not (target / "runs").exists():
        install_examples(workspace)

def install_examples(workspace):
    target = template_root(workspace)/"runs"
    common = OPS_ROOT.parents[1] / "resources/templates/runs"
    for name in PORTABLE_RUNS:
        destination = target/name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(common/name,destination)
    return {"directory":str(target),"installed":list(PORTABLE_RUNS)}
