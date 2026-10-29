from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import os
import re
import subprocess
import time

SHUTDOWN_SECONDS = 120
SIGNAL_MARGIN_SECONDS = 180

TERMINAL = frozenset(
    {
        "COMPLETED",
        "CANCELLED",
        "FAILED",
        "TIMEOUT",
        "NODE_FAIL",
        "OUT_OF_MEMORY",
        "BOOT_FAIL",
        "DEADLINE",
        "PREEMPTED",
        "REVOKED",
    }
)
ACTIVE = frozenset(
    {
        "PENDING",
        "RUNNING",
        "CONFIGURING",
        "COMPLETING",
        "SUSPENDED",
        "RESIZING",
        "REQUEUED",
        "REQUEUE_FED",
        "REQUEUE_HOLD",
        "SIGNALING",
        "STAGE_OUT",
    }
)


class SchedulerError(RuntimeError):
    pass


@dataclass(frozen=True)
class Job:
    id: str
    name: str
    state: str
    node: str
    comment: str

    @property
    def terminal(self):
        return self.state in TERMINAL


def run(args, *, env=None, timeout=30):
    try:
        result = subprocess.run(
            args,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise SchedulerError(f"{args[0]} unavailable: {error}") from error
    if result.returncode:
        raise SchedulerError(f"{args[0]} failed ({result.returncode}): {result.stderr.strip()}")
    return result


def parse_jobs(text):
    jobs = []
    for line in text.splitlines():
        fields = line.split("|", 4)
        if len(fields) != 5:
            raise SchedulerError(f"invalid scheduler response: {line!r}")
        job_id, name, state, node, comment = (field.strip() for field in fields)
        state = state.split()[0].rstrip("+") if state else ""
        if not re.fullmatch(r"\d+(?:_\d+)?", job_id):
            continue  # accounting steps are not allocations
        if state not in ACTIVE | TERMINAL:
            raise SchedulerError(f"unknown Slurm state {state!r}")
        jobs.append(
            Job(
                job_id,
                name,
                state,
                "" if node in ("None", "(null)", "Unknown") else node,
                "" if comment in ("(null)", "None") else comment,
            )
        )
    return jobs


class Slurm:
    def __init__(self, runner=run):
        self.run = runner
        self._snapshot = None

    @contextmanager
    def snapshot(self):
        """One scheduler snapshot per reconciliation pass, not one per worker."""
        previous = self._snapshot
        self._snapshot = (self.active(), {})
        try:
            yield self
        finally:
            self._snapshot = previous

    def active(self):
        if self._snapshot is not None:
            return self._snapshot[0]
        return parse_jobs(
            self.run(
                ["squeue", "-h", "-u", os.environ.get("USER", ""), "-o", "%i|%.128j|%T|%N|%.256k"]
            ).stdout
        )

    def history(self, *, job_id=None, since="1970-01-01"):
        key = (job_id, since)
        if self._snapshot is not None and key in self._snapshot[1]:
            return self._snapshot[1][key]
        args = [
            "sacct",
            "-X",
            "--noheader",
            "--parsable2",
            "--user",
            os.environ.get("USER", ""),
            "--starttime",
            since,
            "--format=JobIDRaw,JobName%128,State%32,NodeList%256,Comment%256",
        ]
        if job_id:
            args += ["--jobs", job_id]
        jobs = parse_jobs(self.run(args).stdout)
        if self._snapshot is not None:
            self._snapshot[1][key] = jobs
        return jobs

    def lookup(self, job_id, since):
        for job in self.active():
            if job.id == job_id:
                return job
        rows = self.history(job_id=None if self._snapshot is not None else job_id, since=since)
        return next(
            (job for job in rows if job.id == job_id), None
        )  # None is UNKNOWN, not terminal

    def find(self, comment, since):
        prefix, deployment, suffix = comment.split(":", 2)
        expected_name = f"{prefix}-{deployment}-{suffix}"
        matches = lambda job: (
            job.name == expected_name and (not job.comment or job.comment == comment)
        )
        active = [job for job in self.active() if matches(job)]
        if active:
            return active
        return [job for job in self.history(since=since) if matches(job)]

    def submit(self, script, *, name, comment, workspace, env, options=()):
        response = self.run(
            [
                "sbatch",
                "--parsable",
                "--chdir",
                str(workspace),
                "--job-name",
                name,
                "--comment",
                comment,
                *options,
                str(script),
            ],
            env=env,
        )
        job_id = response.stdout.strip().split(";", 1)[0]
        if not re.fullmatch(r"\d+", job_id):
            raise SchedulerError(
                f"uncertain submission: unexpected sbatch response {response.stdout!r}"
            )
        return job_id

    def cancel(self, ids):
        if ids:
            self.run(["scancel", *ids])


SBATCH_OPTIONS = {
    name: "--" + name.replace("_", "-")
    for name in (
        "account",
        "partition",
        "qos",
        "wckey",
        "reservation",
        "gres",
        "gpus",
        "cpus_per_task",
        "mem",
        "mem_per_cpu",
        "time",
        "constraint",
        "nodelist",
        "exclude",
    )
}


def options(config):
    unknown = config.keys() - SBATCH_OPTIONS.keys()
    if unknown:
        raise ValueError(f"unknown Slurm resource fields: {sorted(unknown)}")
    if "mem" in config and "mem_per_cpu" in config:
        raise ValueError("choose mem or mem_per_cpu")
    return [f"{option}={config[key]}" for key, option in SBATCH_OPTIONS.items() if key in config]


def time_seconds(value):
    text = str(value)
    days, separator, clock = text.partition("-")
    if not separator:
        clock, days = days, "0"
    fields = clock.split(":")
    if not days.isdigit() or not all(x.isdigit() for x in fields) or not 1 <= len(fields) <= 3:
        raise ValueError("time must be finite Slurm minutes, MM:SS, HH:MM:SS or D-HH:MM:SS")
    parts = list(map(int, fields))
    if separator:
        parts += [0] * (3 - len(parts))
    elif len(parts) == 1:
        parts = [0, parts[0], 0]
    elif len(parts) == 2:
        parts = [0, *parts]
    seconds = int(days) * 86400 + parts[0] * 3600 + parts[1] * 60 + parts[2]
    if seconds <= 0:
        raise ValueError("time must be positive")
    return seconds


def wait_absent(slurm, predicate, timeout):
    deadline = time.monotonic() + timeout
    while True:
        jobs = [job for job in slurm.active() if predicate(job)]
        if not jobs:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError(
                f"jobs still present after shutdown timeout: {[job.id for job in jobs]}"
            )
        time.sleep(min(2, max(0, deadline - time.monotonic())))


def overlay_resources(base, override):
    result = dict(base)
    for present, excluded in [("mem", "mem_per_cpu"), ("mem_per_cpu", "mem")]:
        if present in override:
            result.pop(excluded, None)
    result.update(override)
    options(result)
    return result
