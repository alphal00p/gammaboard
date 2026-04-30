#!/usr/bin/env python3
from __future__ import annotations

import argparse
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
SSH_TARGET = "cs22u040@submit03.unibe.ch"
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


def wait_for_control_node(job_id: str, timeout: int = 180) -> str:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = run(["squeue", "-h", "-j", job_id, "-o", "%N"], check=False)
        node = result.stdout.strip()
        if node and node != "(null)":
            return node
        time.sleep(2)
    raise SystemExit(f"timed out waiting for control node assignment for job {job_id}")


def wait_for_http(url: str, timeout: int = 180) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            urllib.request.urlopen(url, timeout=3).close()
            return
        except (OSError, urllib.error.URLError):
            time.sleep(2)
    raise SystemExit(f"timed out waiting for {url}")


def api_url(control_node: str, path: str) -> str:
    return f"http://{control_node}:{API_PORT}/api{path}"


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


def command_up(args: argparse.Namespace) -> None:
    control = submit_control()
    print(f"control_job_id={control.id}")
    node = wait_for_control_node(control.id, args.startup_timeout)
    print(f"control_node={node}")
    print(f"tunnel=ssh -N -L {args.local_port}:{node}:{FRONTEND_PORT} {SSH_TARGET}")
    wait_for_http(f"http://{node}:{FRONTEND_PORT}", args.startup_timeout)
    print("frontend_ready=true")
    if args.watch:
        print("watching control job; Ctrl-C stops only this launcher")
        try:
            while active_jobs(name=CONTROL_JOB_NAME):
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
    db_url = f"postgresql://postgres:postgres@{control_node}:{DB_PORT}/gammaboard_db"
    ensure_dirs()
    for i in range(1, args.count + 1):
        node_name = f"{args.prefix}-{i}"
        env = os.environ.copy()
        env.update(
            {
                "NODE_NAME": node_name,
                "GAMMABOARD_DATABASE_URL": db_url,
                "GAMMABOARD_IMAGE": IMAGE_PATH,
                "GAMMABOARD_WORKSPACE_ROOT": WORKSPACE_ROOT,
                "DEPLOY_NAME": DEPLOY_NAME,
            }
        )
        result = run(
            ["sbatch", "--chdir", WORKSPACE_ROOT, "--job-name", WORKER_JOB_NAME, WORKER_SBATCH],
            env=env,
        )
        print(f"{node_name}\t{parse_job_id(result.stdout)}")


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
    workers.set_defaults(func=command_submit_workers)

    build = sub.add_parser("build", help="login node: submit a build job")
    build.add_argument("target", choices=("gammaboard", "gammaloop"))
    build.set_defaults(func=command_build)

    return p


def main() -> None:
    args = parser().parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
