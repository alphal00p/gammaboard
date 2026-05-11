#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request
from cmd import PROMPT
from dataclasses import dataclass
from typing import Callable

WORKSPACE_ROOT = "/storage/research/itp_localunitaritydata/gammaboard"
JOB_PREFIX = "gb"
CONTROL_JOB_NAME = f"{JOB_PREFIX}-ctl"
SINGLE_NODE_JOB_NAME = f"{JOB_PREFIX}-single"
WORKER_JOB_NAME = f"{JOB_PREFIX}-wrk"
CONTROL_SBATCH = f"{WORKSPACE_ROOT}/ops/slurm/control.sbatch"
SINGLE_NODE_SBATCH = f"{WORKSPACE_ROOT}/ops/slurm/single_node_deploy.sbatch"
WORKER_SBATCH = f"{WORKSPACE_ROOT}/ops/slurm/worker.sbatch"
GB_BUILD_SBATCH = f"{WORKSPACE_ROOT}/ops/build/gammaboard.sbatch"
GL_BUILD_SBATCH = f"{WORKSPACE_ROOT}/ops/build/gammaloop.sbatch"
IMAGE_PATH = f"{WORKSPACE_ROOT}/images/gammaboard/gammaboard.sif"
DEFAULT_NIX_ROOT = f"/scratch/network/users/{os.environ.get('USER', 'unknown')}/gammaboard-nix"
NIX_ROOT = DEFAULT_NIX_ROOT
NIX_VERSION = "2.24.11"
FRONTEND_PORT = 8080
DB_PORT = 5400
DEPLOY_NAME = "default"
DEFAULT_SSH_HOST = "submit03.unibe.ch"
DEFAULT_ADMIN_PASSWORD = "admin"
DEFAULT_CONTROL_TIME = "00:20:00"
DB_PATH = os.path.join(WORKSPACE_ROOT, "db/default")


def shifted_port(base: int, port_offset: int, label: str) -> int:
    port = base + port_offset
    if port > 65535:
        raise SystemExit(
            f"{label} overflow with port_offset={port_offset} (base={base})"
        )
    return port


def port_offset_from_env() -> int:
    value = os.environ.get("GAMMABOARD_PORT_OFFSET", "0")
    try:
        port_offset = int(value)
    except ValueError as err:
        raise SystemExit(
            f"GAMMABOARD_PORT_OFFSET must be a non-negative integer: {value}"
        ) from err
    if port_offset < 0:
        raise SystemExit(
            f"GAMMABOARD_PORT_OFFSET must be a non-negative integer: {value}"
        )
    return port_offset


def frontend_port(port_offset: int | None = None) -> int:
    resolved = port_offset if port_offset is not None else port_offset_from_env()
    return shifted_port(FRONTEND_PORT, resolved, "frontend port")


def db_port(port_offset: int | None = None) -> int:
    resolved = port_offset if port_offset is not None else port_offset_from_env()
    return shifted_port(DB_PORT, resolved, "postgres port")


def command_clear_db(*_) -> None:
    import shutil

    print(
        f"are you sure you want to delete the local SQLite database at {DB_PATH}? [y/N]",
        file=sys.stderr,
    )
    response = input().strip().lower()
    if response != "y":
        print("aborting", file=sys.stderr)
        return

    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH)

    print("database cleared")


@dataclass(frozen=True)
class Job:
    id: str
    name: str
    state: str
    node: str


def run(
    args: list[str], *, env: dict[str, str] | None = None, check: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )


def http_opener_without_proxies() -> urllib.request.OpenerDirector:
    return urllib.request.build_opener(urllib.request.ProxyHandler({}))


def ensure_dirs() -> None:
    for rel in (
        "logs/slurm/build",
        "logs/slurm/control",
        "logs/slurm/workers",
        "logs/postgres",
    ):
        os.makedirs(os.path.join(WORKSPACE_ROOT, rel), exist_ok=True)


def parse_job_id(output: str) -> str:
    match = re.search(r"Submitted batch job\s+(\d+)", output)
    if not match:
        raise SystemExit(f"failed to parse sbatch output: {output.strip()}")
    return match.group(1)


def active_jobs(name: str | None = None, prefix: str | None = None) -> list[Job]:
    result = run(
        ["squeue", "-h", "-u", os.environ.get("USER", ""), "-o", "%i|%j|%T|%N"],
        check=False,
    )
    jobs: list[Job] = []
    for line in result.stdout.splitlines():
        parts = line.split("|", 3)
        if len(parts) != 4:
            continue
        job = Job(
            id=parts[0].strip(),
            name=parts[1].strip(),
            state=parts[2].strip(),
            node=parts[3].strip(),
        )
        if name is not None and job.name != name:
            continue
        if prefix is not None and not job.name.startswith(prefix):
            continue
        jobs.append(job)
    return jobs


def require_single_control() -> Job:
    jobs = active_jobs(name=CONTROL_JOB_NAME)
    if not jobs:
        raise SystemExit(f"no active control job named {CONTROL_JOB_NAME}")
    if len(jobs) > 1:
        for job in jobs:
            print(f"{job.id}\t{job.name}\t{job.state}\t{job.node}", file=sys.stderr)
        raise SystemExit(f"multiple active control jobs named {CONTROL_JOB_NAME}")
    return jobs[0]


def wait_for_job_node(job_id: str, timeout: int = 180, *, verbose: bool = False) -> str:
    deadline = time.monotonic() + timeout
    next_status = 0.0
    while time.monotonic() < deadline:
        result = run(["squeue", "-h", "-j", job_id, "-o", "%N"], check=False)
        node = result.stdout.strip()
        if node and node != "(null)":
            return node
        now = time.monotonic()
        if verbose and now >= next_status:
            print(f"waiting for Slurm node assignment for job {job_id}")
            next_status = now + 10
        time.sleep(2)
    raise SystemExit(f"timed out waiting for Slurm node assignment for job {job_id}")


def slurm_log_paths(job_name: str, job_id: str) -> tuple[str, str]:
    base = os.path.join(WORKSPACE_ROOT, "logs/slurm/control")
    return (
        os.path.join(base, f"{job_name}-{job_id}.out"),
        os.path.join(base, f"{job_name}-{job_id}.err"),
    )


def job_is_active(job_id: str) -> bool:
    result = run(["squeue", "-h", "-j", job_id, "-o", "%i"], check=False)
    return any(line.strip() == job_id for line in result.stdout.splitlines())


def status_lines() -> list[str]:
    control_jobs = active_jobs(name=CONTROL_JOB_NAME)
    single_node_jobs = active_jobs(name=SINGLE_NODE_JOB_NAME)
    worker_jobs = active_jobs(name=WORKER_JOB_NAME)
    lines: list[str] = []
    if control_jobs:
        for job in control_jobs:
            lines.append(f"control     {job.id:<8} {job.state:<10} {job.node or '-'}")
    else:
        lines.append("control     -        not-running -")
    if single_node_jobs:
        for job in single_node_jobs:
            lines.append(f"single-node {job.id:<8} {job.state:<10} {job.node or '-'}")
    else:
        lines.append("single-node -        not-running -")
    lines.append(f"workers     {len(worker_jobs)}")
    for job in worker_jobs:
        lines.append(f"worker      {job.id:<8} {job.state:<10} {job.node or '-'}")
    return lines


class LiveStatusPrinter:
    def __init__(self) -> None:
        self.enabled = sys.stdout.isatty()
        self.rendered_lines = 0

    def clear(self) -> None:
        if not self.enabled or self.rendered_lines == 0:
            return
        for _ in range(self.rendered_lines):
            sys.stdout.write("\r\033[2K\033[1A")
        sys.stdout.write("\r\033[2K")
        sys.stdout.flush()
        self.rendered_lines = 0

    def print_event(self, line: str) -> None:
        self.clear()
        print(line)

    def render(self, lines: list[str]) -> None:
        if not self.enabled:
            for line in lines:
                print(line)
            return
        self.clear()
        for line in lines:
            print(line)
        self.rendered_lines = len(lines)


def wait_for_http(
    url: str,
    timeout: int = 180,
    *,
    verbose: bool = False,
    job: Job | None = None,
    on_status: Callable[[str], None] | None = None,
) -> None:
    deadline = time.monotonic() + timeout
    next_status = 0.0
    last_error = ""
    opener = http_opener_without_proxies()
    while time.monotonic() < deadline:
        if job is not None and not job_is_active(job.id):
            raise SystemExit(
                f"{job.name} job {job.id} exited before frontend became ready"
            )
        try:
            with opener.open(url, timeout=3) as response:
                status = getattr(response, "status", None) or response.getcode()
            if on_status is not None:
                on_status(f"http_ready status={status} url={url}")
            return
        except (OSError, urllib.error.URLError) as err:
            last_error = str(err)
            now = time.monotonic()
            if verbose and now >= next_status:
                print(f"waiting for frontend at {url}; last_error={last_error}")
                next_status = now + 10
            time.sleep(2)
    raise SystemExit(f"timed out waiting for {url}; last_error={last_error}")


def api_url(control_node: str, path: str, *, port_offset: int | None = None) -> str:
    # Server API binds to loopback on compute nodes; access it through nginx.
    return f"http://{control_node}:{frontend_port(port_offset)}/api{path}"


def ssh_target() -> str:
    if os.environ.get("SSH_TARGET"):
        return os.environ["SSH_TARGET"]
    user = (
        os.environ.get("SSH_USER")
        or os.environ.get("USER")
        or os.environ.get("LOGNAME")
    )
    if not user:
        raise SystemExit("failed to infer SSH user; set SSH_USER or SSH_TARGET")
    host = os.environ.get("SSH_HOST") or DEFAULT_SSH_HOST
    return f"{user}@{host}"


def tunnel_command(control_node: str, local_port: int, *, port_offset: int) -> str:
    return f"ssh -N -L {local_port}:{control_node}:{frontend_port(port_offset)} {ssh_target()}"


def copy_to_clipboard_osc52(text: str) -> bool:
    if not sys.stdout.isatty():
        return False
    encoded = base64.b64encode(text.encode("utf-8")).decode("ascii")
    try:
        sys.stdout.write(f"\033]52;c;{encoded}\a")
        sys.stdout.flush()
        return True
    except OSError:
        return False


def database_url(control_node: str, *, port_offset: int | None = None) -> str:
    return f"postgresql://postgres:postgres@{control_node}:{db_port(port_offset)}/gammaboard_db"


def login(control_node: str, admin_password: str, *, port_offset: int | None = None) -> str:
    data = f'{{"password":"{admin_password}"}}'.encode()
    request = urllib.request.Request(
        api_url(control_node, "/auth/login", port_offset=port_offset),
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with http_opener_without_proxies().open(request, timeout=5) as response:
            cookie = response.headers.get("Set-Cookie", "")
    except urllib.error.HTTPError as err:
        if err.code == 401:
            raise SystemExit(
                "API login failed (401 Unauthorized). "
                "Set --admin-password or GAMMABOARD_ADMIN_PASSWORD."
            ) from err
        raise
    if not cookie:
        raise SystemExit("login did not return a session cookie")
    return cookie.split(";", 1)[0]


def admin_password(args: argparse.Namespace) -> str:
    return (
        getattr(args, "admin_password", None)
        or os.environ.get("GAMMABOARD_ADMIN_PASSWORD")
        or DEFAULT_ADMIN_PASSWORD
    )


def api_request_json(
    control_node: str,
    path: str,
    *,
    method: str = "POST",
    payload: dict | None = None,
    cookie: str | None = None,
    port_offset: int | None = None,
) -> dict:
    headers = {"Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    request = urllib.request.Request(
        api_url(control_node, path, port_offset=port_offset),
        data=json.dumps(payload or {}).encode(),
        headers=headers,
        method=method,
    )
    with http_opener_without_proxies().open(request, timeout=10) as response:
        body = response.read().decode("utf-8")
    return json.loads(body) if body else {}


def post(
    control_node: str,
    path: str,
    *,
    cookie: str | None = None,
    port_offset: int | None = None,
) -> None:
    api_request_json(control_node, path, cookie=cookie, port_offset=port_offset)


def parse_hms(value: str) -> str:
    if not re.fullmatch(r"\d{2}:\d{2}:\d{2}", value):
        raise argparse.ArgumentTypeError("expected HH:MM:SS")
    return value


def parse_port_offset(value: str) -> int:
    try:
        port_offset = int(value)
    except ValueError as err:
        raise argparse.ArgumentTypeError("expected non-negative integer") from err
    if port_offset < 0:
        raise argparse.ArgumentTypeError("expected non-negative integer")
    shifted_port(FRONTEND_PORT, port_offset, "frontend port")
    shifted_port(DB_PORT, port_offset, "postgres port")
    return port_offset


def submit_singleton_job(
    job_name: str, sbatch_path: str, time_limit: str, *, port_offset: int
) -> Job:
    jobs = active_jobs(name=job_name)
    if len(jobs) == 1:
        return jobs[0]
    if len(jobs) > 1:
        for job in jobs:
            print(f"{job.id}\t{job.name}\t{job.state}\t{job.node}", file=sys.stderr)
        raise SystemExit(f"multiple active jobs named {job_name}")

    ensure_dirs()
    env = os.environ.copy()
    env["GAMMABOARD_PORT_OFFSET"] = str(port_offset)
    result = run(
        [
            "sbatch",
            "--chdir",
            WORKSPACE_ROOT,
            "--job-name",
            job_name,
            "--time",
            time_limit,
            sbatch_path,
        ],
        env=env,
    )
    job_id = parse_job_id(result.stdout)
    return Job(id=job_id, name=job_name, state="SUBMITTED", node="")


def submit_control(time_limit: str, *, port_offset: int) -> Job:
    return submit_singleton_job(
        CONTROL_JOB_NAME, CONTROL_SBATCH, time_limit, port_offset=port_offset
    )


def submit_single_node(time_limit: str, *, port_offset: int) -> Job:
    return submit_singleton_job(
        SINGLE_NODE_JOB_NAME, SINGLE_NODE_SBATCH, time_limit, port_offset=port_offset
    )


def claim_launch_request(
    control_node: str, cookie: str, *, port_offset: int | None = None
) -> dict | None:
    response = api_request_json(
        control_node,
        "/node-launch-requests/claim-external",
        cookie=cookie,
        port_offset=port_offset,
    )
    request = response.get("request")
    return request if isinstance(request, dict) else None


def update_launch_request(
    control_node: str,
    cookie: str,
    request_id: int,
    state: str,
    started_count: int,
    result: dict,
    error: str | None = None,
    port_offset: int | None = None,
) -> None:
    api_request_json(
        control_node,
        f"/node-launch-requests/{int(request_id)}/progress",
        payload={
            "state": state,
            "started_count": started_count,
            "result": result,
            "error": error,
        },
        cookie=cookie,
        port_offset=port_offset,
    )


def node_names_for_request(request: dict) -> list[str]:
    count = int(request.get("requested_count") or 0)
    args = request.get("args") or {}
    if not isinstance(args, dict):
        args = {}
    requested_names = args.get("node_names")
    if isinstance(requested_names, list):
        names = [str(name).strip() for name in requested_names if str(name).strip()]
        if len(names) >= count:
            return names[:count]

    prefix = (
        str(request.get("name_prefix") or args.get("name_prefix") or "w").strip() or "w"
    )
    request_id = int(request["id"])
    return [f"{prefix}-{request_id}-{i}" for i in range(1, count + 1)]


def submit_worker(
    node_name: str,
    control_node: str,
    *,
    control_job_id: str,
    port_offset: int | None = None,
    max_start_failures: int = 3,
) -> str:
    resolved_port_offset = (
        port_offset if port_offset is not None else port_offset_from_env()
    )
    ensure_dirs()
    env = os.environ.copy()
    env.update(
        {
            "NODE_NAME": node_name,
            "NODE_MAX_START_FAILURES": str(max_start_failures),
            "GAMMABOARD_DATABASE_URL": database_url(
                control_node, port_offset=resolved_port_offset
            ),
            "GAMMABOARD_IMAGE": IMAGE_PATH,
            "GAMMABOARD_WORKSPACE_ROOT": WORKSPACE_ROOT,
            "DEPLOY_NAME": DEPLOY_NAME,
            "GAMMABOARD_PORT_OFFSET": str(resolved_port_offset),
            "CONTROL_JOB_ID": control_job_id,
        }
    )
    result = run(
        [
            "sbatch",
            "--chdir",
            WORKSPACE_ROOT,
            "--job-name",
            WORKER_JOB_NAME,
            WORKER_SBATCH,
        ],
        env=env,
    )
    return parse_job_id(result.stdout)


def resolve_launch_requests(
    control: Job,
    control_node: str,
    cookie: str,
    *,
    max_requests: int | None = None,
    port_offset: int | None = None,
) -> int:
    return resolve_launch_requests_with_callback(
        control,
        control_node,
        cookie,
        max_requests=max_requests,
        port_offset=port_offset,
    )


def resolve_launch_requests_with_callback(
    control: Job,
    control_node: str,
    cookie: str,
    *,
    max_requests: int | None = None,
    on_launch: Callable[[str], None] | None = None,
    port_offset: int | None = None,
) -> int:
    resolved_port_offset = (
        port_offset if port_offset is not None else port_offset_from_env()
    )
    resolved = 0
    while max_requests is None or resolved < max_requests:
        request = claim_launch_request(
            control_node, cookie, port_offset=resolved_port_offset
        )
        if request is None:
            break

        request_id = int(request["id"])
        node_names = node_names_for_request(request)
        args = request.get("args") or {}
        if not isinstance(args, dict):
            args = {}
        max_start_failures = int(args.get("max_start_failures") or 3)
        submitted: list[dict[str, str]] = []
        try:
            if not node_names:
                raise RuntimeError("launch request requested zero workers")
            for node_name in node_names:
                job_id = submit_worker(
                    node_name,
                    control_node,
                    control_job_id=control.id,
                    port_offset=resolved_port_offset,
                    max_start_failures=max_start_failures,
                )
                submitted.append({"node_name": node_name, "job_id": job_id})
                message = f"launch_request={request_id}\tnode={node_name}\tjob={job_id}"
                if on_launch is not None:
                    on_launch(message)
                else:
                    print(message)
            update_launch_request(
                control_node,
                cookie,
                request_id,
                "starting",
                len(submitted),
                {"workers": submitted},
                port_offset=resolved_port_offset,
            )
        except Exception as err:
            try:
                update_launch_request(
                    control_node,
                    cookie,
                    request_id,
                    "failed",
                    len(submitted),
                    {"workers": submitted},
                    str(err),
                    port_offset=resolved_port_offset,
                )
            finally:
                print(f"launch_request={request_id}\tfailed={err}", file=sys.stderr)
        resolved += 1
    return resolved


def watch_launch_requests(
    args: argparse.Namespace, control: Job, control_node: str
) -> None:
    print(
        f"watching launch requests on {control_node}; Ctrl-C stops only this launcher"
    )
    cookie = login(control_node, admin_password(args), port_offset=args.port_offset)
    try:
        while True:
            resolved = resolve_launch_requests(
                control, control_node, cookie, port_offset=args.port_offset
            )
            if args.once:
                print(f"resolved_requests={resolved}")
                return
            time.sleep(args.poll_seconds)
    except KeyboardInterrupt:
        print("launcher stopped")


def command_up(args: argparse.Namespace) -> None:
    port_offset = args.port_offset
    job = (
        submit_single_node(args.time, port_offset=port_offset)
        if args.single_node
        else submit_control(args.time, port_offset=port_offset)
    )
    print(f"{'single_node_job_id' if args.single_node else 'control_job_id'}={job.id}")
    node = wait_for_job_node(job.id, args.startup_timeout, verbose=True)
    print(f"control_node={node}")
    tunnel = tunnel_command(node, args.local_port, port_offset=port_offset)
    print(f"tunnel={tunnel}")
    if args.copy:
        print(
            f"copied_to_clipboard={'true' if copy_to_clipboard_osc52(tunnel) else 'false'}"
        )
    wait_for_http(
        f"http://{node}:{frontend_port(port_offset)}",
        args.startup_timeout,
        verbose=True,
        job=job,
        on_status=print,
    )
    print("frontend_ready=true")
    if args.watch:
        print(f"watching {job.name} job; Ctrl-C stops only this launcher")
        status_printer = LiveStatusPrinter()
        try:
            cookie: str | None = None
            if not args.single_node:
                try:
                    cookie = login(node, admin_password(args), port_offset=port_offset)
                except SystemExit as err:
                    print(
                        f"warning: {err}; continuing watch without launch-request resolution",
                        file=sys.stderr,
                    )
            while job_is_active(job.id):
                if not args.single_node and cookie:
                    resolve_launch_requests_with_callback(
                        job,
                        node,
                        cookie,
                        on_launch=status_printer.print_event,
                        port_offset=port_offset,
                    )
                status_printer.render(status_lines())
                time.sleep(args.poll_seconds)
            status_printer.clear()
        except KeyboardInterrupt:
            status_printer.clear()
            print("launcher stopped")


def command_down(args: argparse.Namespace) -> None:
    port_offset = args.port_offset
    deploy_jobs = active_jobs(name=CONTROL_JOB_NAME) + active_jobs(
        name=SINGLE_NODE_JOB_NAME
    )
    if not deploy_jobs:
        raise SystemExit(f"no active {CONTROL_JOB_NAME} or {SINGLE_NODE_JOB_NAME} job")
    if len(deploy_jobs) > 1:
        for job in deploy_jobs:
            print(f"{job.id}\t{job.name}\t{job.state}\t{job.node}", file=sys.stderr)
        raise SystemExit(
            "multiple active deploy jobs; cancel the unwanted job explicitly or retry when only one remains"
        )

    job = deploy_jobs[0]
    node = wait_for_job_node(job.id, args.startup_timeout)
    print(f"job_id={job.id}")
    print(f"job_name={job.name}")
    print(f"control_node={node}")

    try:
        cookie = login(node, admin_password(args), port_offset=port_offset)
        post(node, "/nodes/stop-all", cookie=cookie, port_offset=port_offset)
        print("requested node stop through API")
    except Exception as err:
        print(f"warning: API node stop failed: {err}", file=sys.stderr)

    if job.name == CONTROL_JOB_NAME:
        deadline = time.monotonic() + args.worker_timeout
        while time.monotonic() < deadline:
            workers = active_jobs(name=WORKER_JOB_NAME)
            if not workers:
                break
            print(f"waiting for workers to exit: {len(workers)} active")
            time.sleep(5)

        workers = active_jobs(name=WORKER_JOB_NAME)
        if workers:
            print(
                f"canceling remaining workers: {', '.join(worker.id for worker in workers)}"
            )
            run(["scancel", *[worker.id for worker in workers]], check=False)

    time.sleep(args.control_grace_seconds)
    print(f"canceling {job.name} job {job.id}")
    run(["scancel", job.id], check=False)


def command_status(_: argparse.Namespace) -> None:
    for line in status_lines():
        print(line)


def command_submit_workers(args: argparse.Namespace) -> None:
    control = require_single_control()
    control_node = wait_for_job_node(control.id)
    ensure_dirs()
    for i in range(1, args.count + 1):
        node_name = f"{args.prefix}-{i}"
        job_id = submit_worker(
            node_name,
            control_node,
            control_job_id=control.id,
            port_offset=args.port_offset,
            max_start_failures=args.max_start_failures,
        )
        print(f"{node_name}\t{job_id}")


def command_watch_requests(args: argparse.Namespace) -> None:
    control = require_single_control()
    control_node = wait_for_job_node(control.id, args.startup_timeout)
    watch_launch_requests(args, control, control_node)


def command_build(args: argparse.Namespace) -> None:
    ensure_dirs()
    target = GB_BUILD_SBATCH if args.target == "gammaboard" else GL_BUILD_SBATCH
    result = run(["sbatch", "--chdir", WORKSPACE_ROOT, target])
    print(result.stdout.strip())


def normalize_nix_flake_ref(reference: str) -> str:
    if "://" in reference or reference.startswith(
        ("path:", "github:", "git+", "tarball+", "flake:")
    ):
        return reference
    if reference.startswith(("./", "../", "/")):
        path, sep, fragment = reference.partition("#")
        absolute = path if os.path.isabs(path) else os.path.join(WORKSPACE_ROOT, path)
        return f"path:{absolute}{sep}{fragment}"
    if "/" in reference:
        path, sep, fragment = reference.partition("#")
        return f"path:{os.path.join(WORKSPACE_ROOT, path)}{sep}{fragment}"
    return reference


def local_nix_flake_path(reference: str) -> tuple[str, str] | None:
    if "://" in reference or reference.startswith(("github:", "git+", "tarball+", "flake:")):
        return None
    raw = reference.removeprefix("path:")
    path, _sep, fragment = raw.partition("#")
    if raw == reference and "/" not in path and not path.startswith((".", "/")):
        return None
    absolute = path if os.path.isabs(path) else os.path.join(WORKSPACE_ROOT, path)
    return absolute, fragment


def command_nix_build(args: argparse.Namespace) -> None:
    os.makedirs(NIX_ROOT, exist_ok=True)
    if not os.path.isfile(IMAGE_PATH):
        raise SystemExit(f"missing image: {IMAGE_PATH}")

    ensure_nix = f"""
export PATH="$HOME/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$PATH"
if ! command -v nix >/dev/null 2>&1; then
  echo "installing Nix {NIX_VERSION} into /nix" >&2
  rm -rf /tmp/gammaboard-nix-installer
  mkdir -p /tmp/gammaboard-nix-installer
  curl -fsSL https://releases.nixos.org/nix/nix-{NIX_VERSION}/nix-{NIX_VERSION}-x86_64-linux.tar.xz \
    -o /tmp/gammaboard-nix-installer/nix.tar.xz
  tar -xJf /tmp/gammaboard-nix-installer/nix.tar.xz -C /tmp/gammaboard-nix-installer --strip-components=1
  NIX_INSTALLER_NO_MODIFY_PROFILE=1 /tmp/gammaboard-nix-installer/install --no-daemon
  rm -rf /tmp/gammaboard-nix-installer
fi
if [ -f "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
  . "$HOME/.nix-profile/etc/profile.d/nix.sh"
fi
export PATH="$HOME/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$PATH"
export NIX_CONFIG="experimental-features = nix-command flakes${{NIX_CONFIG:+
$NIX_CONFIG}}"
"""
    local_flake = local_nix_flake_path(args.flake_ref)
    if local_flake is not None:
        source_path, fragment = local_flake
        if not os.path.isdir(source_path):
            raise SystemExit(f"missing flake source directory: {source_path}")
        staged_ref = "/tmp/gammaboard-nix-build-source"
        if fragment:
            staged_ref = f"{staged_ref}#{fragment}"
        nix_command = (
            ensure_nix
            + "\n"
            "rm -rf /tmp/gammaboard-nix-build-source && "
            "mkdir -p /tmp/gammaboard-nix-build-source && "
            f"cp -R --no-preserve=mode,ownership,timestamps,xattr {shlex.quote(source_path)}/. /tmp/gammaboard-nix-build-source/ && "
            f"nix build {shlex.quote(f'path:{staged_ref}')} "
            + " ".join(shlex.quote(arg) for arg in args.nix_args)
        )
        command = [
            "apptainer",
            "exec",
            "-B",
            WORKSPACE_ROOT,
            "-B",
            f"{NIX_ROOT}:/nix",
            IMAGE_PATH,
            "sh",
            "-lc",
            nix_command,
        ]
    else:
        flake_ref = normalize_nix_flake_ref(args.flake_ref)
        nix_command = (
            ensure_nix
            + "\n"
            + f"nix build {shlex.quote(flake_ref)} "
            + " ".join(shlex.quote(arg) for arg in args.nix_args)
        )
        command = [
            "apptainer",
            "exec",
            "-B",
            WORKSPACE_ROOT,
            "-B",
            f"{NIX_ROOT}:/nix",
            IMAGE_PATH,
            "sh",
            "-lc",
            nix_command,
        ]
    result = subprocess.run(command, text=True)
    if result.returncode != 0:
        raise SystemExit(result.returncode)


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=(
            "UBELIX launcher for Gammaboard. Run all commands on a UBELIX login node. "
            "'up' prints the manual SSH tunnel command for your local machine."
        )
    )
    sub = p.add_subparsers(dest="command", required=True)

    up = sub.add_parser("up", help="login node: submit or reuse a deploy job")
    up.add_argument(
        "--watch",
        action="store_true",
        help="block and print status until the deploy job exits",
    )
    up.add_argument(
        "--time",
        type=parse_hms,
        default=DEFAULT_CONTROL_TIME,
        help="Slurm walltime for a newly submitted deploy job (HH:MM:SS)",
    )
    up.add_argument(
        "--single-node",
        action="store_true",
        help="run control and local workers in one Slurm allocation",
    )
    up.add_argument("--local-port", type=int, default=8080)
    up.add_argument("--port-offset", type=parse_port_offset, default=0)
    up.add_argument("--startup-timeout", type=int, default=180)
    up.add_argument("--poll-seconds", type=int, default=15)
    up.add_argument(
        "--copy",
        action="store_true",
        help="copy the SSH tunnel command to the clipboard if supported",
    )
    up.add_argument(
        "--admin-password",
        default=None,
        help="dashboard admin password (or set GAMMABOARD_ADMIN_PASSWORD)",
    )
    up.set_defaults(func=command_up)

    down = sub.add_parser(
        "down", help="login node: gracefully stop nodes, then cancel remaining jobs"
    )
    down.add_argument("--startup-timeout", type=int, default=60)
    down.add_argument("--port-offset", type=parse_port_offset, default=0)
    down.add_argument("--worker-timeout", type=int, default=60)
    down.add_argument("--control-grace-seconds", type=int, default=5)
    down.add_argument(
        "--admin-password",
        default=None,
        help="dashboard admin password (or set GAMMABOARD_ADMIN_PASSWORD)",
    )
    down.set_defaults(func=command_down)

    status = sub.add_parser(
        "status", help="login node: show active deploy and worker jobs"
    )
    status.set_defaults(func=command_status)

    workers = sub.add_parser(
        "submit-workers", help="login node: submit N separate worker jobs"
    )
    workers.add_argument("--count", type=int, required=True)
    workers.add_argument("--prefix", default="w")
    workers.add_argument("--port-offset", type=parse_port_offset, default=0)
    workers.add_argument("--max-start-failures", type=int, default=3)
    workers.set_defaults(func=command_submit_workers)

    watch_requests = sub.add_parser(
        "watch-requests",
        help="login node: resolve pending DB node launch requests into Slurm worker jobs",
    )
    watch_requests.add_argument(
        "--once", action="store_true", help="resolve current pending requests and exit"
    )
    watch_requests.add_argument("--startup-timeout", type=int, default=60)
    watch_requests.add_argument("--port-offset", type=parse_port_offset, default=0)
    watch_requests.add_argument("--poll-seconds", type=int, default=5)
    watch_requests.add_argument(
        "--admin-password",
        default=None,
        help="dashboard admin password (or set GAMMABOARD_ADMIN_PASSWORD)",
    )
    watch_requests.set_defaults(func=command_watch_requests)

    build = sub.add_parser("build", help="login node: submit a build job")
    build.add_argument("target", choices=("gammaboard", "gammaloop"))
    build.set_defaults(func=command_build)

    nix_build = sub.add_parser(
        "nix-build",
        help="login node: run nix build inside the GammaBoard image with the persistent /nix bind",
    )
    nix_build.add_argument(
        "flake_ref",
        help="flake output to build; relative paths resolve under the UBELIX workspace",
    )
    nix_build.add_argument("nix_args", nargs=argparse.REMAINDER)
    nix_build.set_defaults(func=command_nix_build)

    clear = sub.add_parser(
        "clear-db", help="login node: delete the local SQLite database"
    )
    clear.set_defaults(func=command_clear_db)

    return p


def main() -> None:
    args = parser().parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
