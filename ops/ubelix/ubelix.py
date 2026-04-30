#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass


WORKSPACE_ROOT = "/storage/research/itp_localunitaritydata"
JOB_PREFIX = "gb-default"
CONTROL_JOB_NAME = f"{JOB_PREFIX}-control"
WORKER_JOB_NAME = f"{JOB_PREFIX}-worker"
CONTROL_SBATCH = f"{WORKSPACE_ROOT}/ops/slurm/control_ui_single.sbatch"
WORKER_SBATCH = f"{WORKSPACE_ROOT}/ops/slurm/node_worker.sbatch"
GB_BUILD_SBATCH = f"{WORKSPACE_ROOT}/ops/build/build_latest_gammaboard.sbatch"
GL_BUILD_SBATCH = f"{WORKSPACE_ROOT}/ops/build/build_latest_gammaloop.sbatch"
IMAGE_PATH = f"{WORKSPACE_ROOT}/images/gammaboard/gammaboard-latest.sif"
FRONTEND_PORT = 8080
API_PORT = 4000
DB_PORT = 5433
DEPLOY_NAME = "default"
DEFAULT_SSH_HOST = "submit03.unibe.ch"
ADMIN_PASSWORD = "admin"


@dataclass(frozen=True)
class Job:
    id: str
    name: str
    state: str
    node: str


def run(args: list[str], *, env: dict[str, str] | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )


def ensure_dirs() -> None:
    for rel in ("logs/slurm/build", "logs/slurm/control", "logs/slurm/workers", "logs/postgres"):
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
        job = Job(id=parts[0].strip(), name=parts[1].strip(), state=parts[2].strip(), node=parts[3].strip())
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


def wait_for_control_node(job_id: str, timeout: int = 180, *, verbose: bool = False) -> str:
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
    raise SystemExit(f"timed out waiting for control node assignment for job {job_id}")


def slurm_log_paths(job_id: str) -> tuple[str, str]:
    base = os.path.join(WORKSPACE_ROOT, "logs/slurm/control")
    return (
        os.path.join(base, f"{CONTROL_JOB_NAME}-{job_id}.out"),
        os.path.join(base, f"{CONTROL_JOB_NAME}-{job_id}.err"),
    )


def tail_file(path: str, lines: int = 80) -> str:
    if not os.path.exists(path):
        return f"{path}: missing"
    result = run(["tail", "-n", str(lines), path], check=False)
    return result.stdout.rstrip() or f"{path}: empty"


def print_control_log_tail(job_id: str) -> None:
    out_path, err_path = slurm_log_paths(job_id)
    print(f"--- {out_path} ---", file=sys.stderr)
    print(tail_file(out_path), file=sys.stderr)
    print(f"--- {err_path} ---", file=sys.stderr)
    print(tail_file(err_path), file=sys.stderr)


def job_is_active(job_id: str) -> bool:
    result = run(["squeue", "-h", "-j", job_id, "-o", "%i"], check=False)
    return any(line.strip() == job_id for line in result.stdout.splitlines())


def wait_for_http(
    url: str,
    timeout: int = 180,
    *,
    verbose: bool = False,
    control_job_id: str | None = None,
) -> None:
    deadline = time.monotonic() + timeout
    next_status = 0.0
    last_error = ""
    while time.monotonic() < deadline:
        if control_job_id is not None and not job_is_active(control_job_id):
            print_control_log_tail(control_job_id)
            raise SystemExit(f"control job {control_job_id} exited before frontend became ready")
        try:
            urllib.request.urlopen(url, timeout=3).close()
            return
        except (OSError, urllib.error.URLError) as err:
            last_error = str(err)
            now = time.monotonic()
            if verbose and now >= next_status:
                print(f"waiting for frontend at {url}; last_error={last_error}")
                next_status = now + 10
            time.sleep(2)
    if control_job_id is not None:
        print_control_log_tail(control_job_id)
    raise SystemExit(f"timed out waiting for {url}; last_error={last_error}")


def api_url(control_node: str, path: str) -> str:
    return f"http://{control_node}:{API_PORT}/api{path}"


def ssh_target() -> str:
    if os.environ.get("SSH_TARGET"):
        return os.environ["SSH_TARGET"]
    user = os.environ.get("SSH_USER") or os.environ.get("USER") or os.environ.get("LOGNAME")
    if not user:
        raise SystemExit("failed to infer SSH user; set SSH_USER or SSH_TARGET")
    host = os.environ.get("SSH_HOST") or DEFAULT_SSH_HOST
    return f"{user}@{host}"


def database_url(control_node: str) -> str:
    return f"postgresql://postgres:postgres@{control_node}:{DB_PORT}/gammaboard_db"


def login(control_node: str) -> str:
    data = f'{{"password":"{ADMIN_PASSWORD}"}}'.encode()
    request = urllib.request.Request(
        api_url(control_node, "/auth/login"),
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        cookie = response.headers.get("Set-Cookie", "")
    if not cookie:
        raise SystemExit("login did not return a session cookie")
    return cookie.split(";", 1)[0]


def post(control_node: str, path: str, *, cookie: str | None = None) -> None:
    headers = {"Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    request = urllib.request.Request(
        api_url(control_node, path),
        data=b"{}",
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=10):
        return


def submit_control() -> Job:
    jobs = active_jobs(name=CONTROL_JOB_NAME)
    if len(jobs) == 1:
        return jobs[0]
    if len(jobs) > 1:
        for job in jobs:
            print(f"{job.id}\t{job.name}\t{job.state}\t{job.node}", file=sys.stderr)
        raise SystemExit(f"multiple active control jobs named {CONTROL_JOB_NAME}")

    ensure_dirs()
    result = run(["sbatch", "--chdir", WORKSPACE_ROOT, "--job-name", CONTROL_JOB_NAME, CONTROL_SBATCH])
    job_id = parse_job_id(result.stdout)
    return Job(id=job_id, name=CONTROL_JOB_NAME, state="SUBMITTED", node="")


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def psql_json(control_node: str, inner_sql: str) -> list[dict]:
    env = os.environ.copy()
    env.pop("PGTZ", None)
    env.pop("PGOPTIONS", None)
    env.setdefault("APPTAINERENV_LANG", "C.UTF-8")
    env.setdefault("APPTAINERENV_LC_ALL", "C.UTF-8")
    env.setdefault("APPTAINERENV_TZ", "Etc/UTC")
    sql = f"""
COPY (
  SELECT COALESCE(json_agg(row_to_json(q)), '[]'::json)
  FROM (
    {inner_sql}
  ) q
) TO STDOUT
"""
    result = run(
        [
            "apptainer",
            "exec",
            "-B",
            WORKSPACE_ROOT,
            IMAGE_PATH,
            "psql",
            database_url(control_node),
            "-X",
            "-qAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ],
        env=env,
    )
    payload = result.stdout.strip()
    if not payload:
        return []
    parsed = json.loads(payload)
    if not isinstance(parsed, list):
        raise SystemExit(f"expected JSON array from psql, got: {payload}")
    return parsed


def claim_launch_request(control_node: str) -> dict | None:
    rows = psql_json(
        control_node,
        """
        WITH next_request AS (
          SELECT id
          FROM node_launch_requests
          WHERE state = 'pending'
            AND backend = 'external'
          ORDER BY created_at, id
          LIMIT 1
          FOR UPDATE SKIP LOCKED
        )
        UPDATE node_launch_requests request
        SET state = 'launching',
            updated_at = now()
        FROM next_request
        WHERE request.id = next_request.id
        RETURNING
          request.id,
          request.backend,
          request.requested_count,
          request.started_count,
          request.name_prefix,
          request.args
        """,
    )
    return rows[0] if rows else None


def update_launch_request(
    control_node: str,
    request_id: int,
    state: str,
    started_count: int,
    result: dict,
    error: str | None = None,
) -> None:
    error_sql = "NULL" if error is None else sql_literal(error)
    psql_json(
        control_node,
        f"""
        UPDATE node_launch_requests
        SET state = {sql_literal(state)},
            started_count = {started_count},
            result = {sql_literal(json.dumps(result, sort_keys=True))}::jsonb,
            error = {error_sql},
            updated_at = now()
        WHERE id = {int(request_id)}
        RETURNING id
        """,
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

    prefix = str(request.get("name_prefix") or args.get("name_prefix") or "w").strip() or "w"
    request_id = int(request["id"])
    return [f"{prefix}-{request_id}-{i}" for i in range(1, count + 1)]


def submit_worker(node_name: str, control_node: str, *, max_start_failures: int = 3) -> str:
    ensure_dirs()
    env = os.environ.copy()
    env.update(
        {
            "NODE_NAME": node_name,
            "NODE_MAX_START_FAILURES": str(max_start_failures),
            "GAMMABOARD_DATABASE_URL": database_url(control_node),
            "GAMMABOARD_IMAGE": IMAGE_PATH,
            "GAMMABOARD_WORKSPACE_ROOT": WORKSPACE_ROOT,
            "DEPLOY_NAME": DEPLOY_NAME,
        }
    )
    result = run(
        ["sbatch", "--chdir", WORKSPACE_ROOT, "--job-name", WORKER_JOB_NAME, WORKER_SBATCH],
        env=env,
    )
    return parse_job_id(result.stdout)


def resolve_one_launch_request(control_node: str) -> bool:
    request = claim_launch_request(control_node)
    if request is None:
        return False

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
            job_id = submit_worker(node_name, control_node, max_start_failures=max_start_failures)
            submitted.append({"node_name": node_name, "job_id": job_id})
            print(f"launch_request={request_id}\tnode={node_name}\tjob={job_id}")
        update_launch_request(
            control_node,
            request_id,
            "succeeded",
            len(submitted),
            {"workers": submitted},
        )
    except Exception as err:
        try:
            update_launch_request(
                control_node,
                request_id,
                "failed",
                len(submitted),
                {"workers": submitted},
                str(err),
            )
        finally:
            print(f"launch_request={request_id}\tfailed={err}", file=sys.stderr)
    return True


def resolve_launch_requests(control_node: str, *, max_requests: int | None = None) -> int:
    resolved = 0
    while max_requests is None or resolved < max_requests:
        if not resolve_one_launch_request(control_node):
            break
        resolved += 1
    return resolved


def watch_launch_requests(args: argparse.Namespace, control_node: str) -> None:
    print(f"watching launch requests on {control_node}; Ctrl-C stops only this launcher")
    try:
        while True:
            resolved = resolve_launch_requests(control_node)
            if args.once:
                print(f"resolved_requests={resolved}")
                return
            time.sleep(args.poll_seconds)
    except KeyboardInterrupt:
        print("launcher stopped")


def command_up(args: argparse.Namespace) -> None:
    control = submit_control()
    print(f"control_job_id={control.id}")
    node = wait_for_control_node(control.id, args.startup_timeout, verbose=True)
    print(f"control_node={node}")
    print(f"tunnel=ssh -N -L {args.local_port}:{node}:{FRONTEND_PORT} {ssh_target()}")
    if not args.no_wait:
        wait_for_http(
            f"http://{node}:{FRONTEND_PORT}",
            args.startup_timeout,
            verbose=True,
            control_job_id=control.id,
        )
        print("frontend_ready=true")
    if args.watch:
        print("watching control job; Ctrl-C stops only this launcher")
        try:
            while active_jobs(name=CONTROL_JOB_NAME):
                resolve_launch_requests(node)
                command_status(argparse.Namespace())
                time.sleep(args.poll_seconds)
        except KeyboardInterrupt:
            print("launcher stopped")


def command_down(args: argparse.Namespace) -> None:
    control = require_single_control()
    node = wait_for_control_node(control.id, args.startup_timeout)
    print(f"control_job_id={control.id}")
    print(f"control_node={node}")

    try:
        cookie = login(node)
        post(node, "/nodes/stop-all", cookie=cookie)
        print("requested node stop through API")
    except Exception as err:
        print(f"warning: API node stop failed: {err}", file=sys.stderr)

    deadline = time.monotonic() + args.worker_timeout
    while time.monotonic() < deadline:
        workers = active_jobs(name=WORKER_JOB_NAME)
        if not workers:
            break
        print(f"waiting for workers to exit: {len(workers)} active")
        time.sleep(5)

    workers = active_jobs(name=WORKER_JOB_NAME)
    if workers:
        print(f"canceling remaining workers: {', '.join(job.id for job in workers)}")
        run(["scancel", *[job.id for job in workers]], check=False)

    time.sleep(args.control_grace_seconds)
    print(f"canceling control job {control.id}")
    run(["scancel", control.id], check=False)


def command_status(_: argparse.Namespace) -> None:
    control_jobs = active_jobs(name=CONTROL_JOB_NAME)
    worker_jobs = active_jobs(name=WORKER_JOB_NAME)
    if control_jobs:
        for job in control_jobs:
            print(f"control\t{job.id}\t{job.state}\t{job.node}")
    else:
        print("control\t-\tnot-running\t-")
    print(f"workers\t{len(worker_jobs)}")
    for job in worker_jobs:
        print(f"worker\t{job.id}\t{job.state}\t{job.node}")


def command_submit_workers(args: argparse.Namespace) -> None:
    control = require_single_control()
    control_node = wait_for_control_node(control.id)
    ensure_dirs()
    for i in range(1, args.count + 1):
        node_name = f"{args.prefix}-{i}"
        print(
            f"{node_name}\t"
            f"{submit_worker(node_name, control_node, max_start_failures=args.max_start_failures)}"
        )


def command_watch_requests(args: argparse.Namespace) -> None:
    control = require_single_control()
    control_node = wait_for_control_node(control.id, args.startup_timeout)
    watch_launch_requests(args, control_node)


def command_build(args: argparse.Namespace) -> None:
    ensure_dirs()
    target = GB_BUILD_SBATCH if args.target == "gammaboard" else GL_BUILD_SBATCH
    result = run(["sbatch", "--chdir", WORKSPACE_ROOT, target])
    print(result.stdout.strip())


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=(
            "UBELIX launcher for Gammaboard. Run all commands on a UBELIX login node. "
            "'up' prints the manual SSH tunnel command for your local machine."
        )
    )
    sub = p.add_subparsers(dest="command", required=True)

    up = sub.add_parser("up", help="login node: submit or reuse the control/UI job")
    up.add_argument("--watch", action="store_true", help="block and print status until control exits")
    up.add_argument("--local-port", type=int, default=8080)
    up.add_argument("--startup-timeout", type=int, default=180)
    up.add_argument("--poll-seconds", type=int, default=15)
    up.add_argument("--no-wait", action="store_true", help="print the tunnel command without waiting for nginx")
    up.set_defaults(func=command_up)

    down = sub.add_parser("down", help="login node: gracefully stop nodes, then cancel remaining jobs")
    down.add_argument("--startup-timeout", type=int, default=60)
    down.add_argument("--worker-timeout", type=int, default=60)
    down.add_argument("--control-grace-seconds", type=int, default=5)
    down.set_defaults(func=command_down)

    status = sub.add_parser("status", help="login node: show active control and worker jobs")
    status.set_defaults(func=command_status)

    workers = sub.add_parser("submit-workers", help="login node: submit N separate worker jobs")
    workers.add_argument("--count", type=int, required=True)
    workers.add_argument("--prefix", default="w")
    workers.add_argument("--max-start-failures", type=int, default=3)
    workers.set_defaults(func=command_submit_workers)

    watch_requests = sub.add_parser(
        "watch-requests",
        help="login node: resolve pending DB node launch requests into Slurm worker jobs",
    )
    watch_requests.add_argument("--once", action="store_true", help="resolve current pending requests and exit")
    watch_requests.add_argument("--startup-timeout", type=int, default=60)
    watch_requests.add_argument("--poll-seconds", type=int, default=5)
    watch_requests.set_defaults(func=command_watch_requests)

    build = sub.add_parser("build", help="login node: submit a build job")
    build.add_argument("target", choices=("gammaboard", "gammaloop"))
    build.set_defaults(func=command_build)

    return p


def main() -> None:
    args = parser().parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
