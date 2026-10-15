from __future__ import annotations

from dataclasses import dataclass
import os
import re
import subprocess

TERMINAL = frozenset({'COMPLETED', 'CANCELLED', 'FAILED', 'TIMEOUT', 'NODE_FAIL',
                      'OUT_OF_MEMORY', 'BOOT_FAIL', 'DEADLINE', 'PREEMPTED', 'REVOKED'})
ACTIVE = frozenset({'PENDING', 'RUNNING', 'CONFIGURING', 'COMPLETING', 'SUSPENDED',
                    'RESIZING', 'REQUEUED', 'REQUEUE_FED', 'REQUEUE_HOLD', 'SIGNALING', 'STAGE_OUT'})

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
        result = subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise SchedulerError(f'{args[0]} unavailable: {error}') from error
    if result.returncode:
        raise SchedulerError(f'{args[0]} failed ({result.returncode}): {result.stderr.strip()}')
    return result


def parse_jobs(text):
    jobs = []
    for line in text.splitlines():
        fields = line.split('|', 4)
        if len(fields) != 5:
            raise SchedulerError(f'invalid scheduler response: {line!r}')
        job_id, name, state, node, comment = (field.strip() for field in fields)
        state = state.split()[0].rstrip('+') if state else ''
        if not re.fullmatch(r'\d+(?:_\d+)?', job_id):
            continue  # accounting steps are not allocations
        if state not in ACTIVE | TERMINAL:
            raise SchedulerError(f'unknown Slurm state {state!r}')
        jobs.append(Job(job_id, name, state, '' if node in ('None', '(null)', 'Unknown') else node, '' if comment in ('(null)', 'None') else comment))
    return jobs


class Slurm:
    def __init__(self, runner=run):
        self.run = runner

    def active(self):
        return parse_jobs(self.run(['squeue', '-h', '-u', os.environ.get('USER', ''),
                                    '-o', '%i|%.128j|%T|%N|%.256k']).stdout)

    def history(self, *, job_id=None, since='1970-01-01'):
        args = ['sacct', '-X', '--noheader', '--parsable2', '--user', os.environ.get('USER', ''),
                '--starttime', since, '--format=JobIDRaw,JobName%128,State%32,NodeList%256,Comment%256']
        if job_id:
            args += ['--jobs', job_id]
        return parse_jobs(self.run(args).stdout)

    def lookup(self, job_id, since):
        for job in self.active():
            if job.id == job_id:
                return job
        rows = self.history(job_id=job_id, since=since)
        return next((job for job in rows if job.id == job_id), None)  # None is UNKNOWN, not terminal

    def find(self, comment, since):
        prefix, deployment, suffix = comment.split(':', 2)
        expected_name = f'{prefix}-{deployment}-{suffix}'
        matches = lambda job: job.name == expected_name and (not job.comment or job.comment == comment)
        active = [job for job in self.active() if matches(job)]
        if active:
            return active
        return [job for job in self.history(since=since) if matches(job)]

    def submit(self, script, *, name, comment, workspace, env, options=()):
        response = self.run(['sbatch', '--parsable', '--chdir', str(workspace), '--job-name', name,
                             '--comment', comment, *options, str(script)], env=env)
        job_id = response.stdout.strip().split(';', 1)[0]
        if not re.fullmatch(r'\d+', job_id):
            raise SchedulerError(f'uncertain submission: unexpected sbatch response {response.stdout!r}')
        return job_id

    def cancel(self, ids):
        if ids:
            self.run(['scancel', *ids])
