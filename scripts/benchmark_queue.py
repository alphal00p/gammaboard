#!/usr/bin/env python3
"""Run the synthetic queue suite and print results (Python 3.11+, psql, prebuilt binary)."""
import argparse
import itertools
import json
import math
from pathlib import Path
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MAX_SECONDS = 300
WARMUP_SECONDS = 0.5


def cases(evaluators=(1, 4, 16, 64), rates=(1000, 2000000), regimes=('inference', 'training_burst')):
    return list(itertools.product(evaluators, rates, regimes))


def inline(value):
    if isinstance(value, dict):
        return '{ ' + ', '.join(f'{key} = {inline(item)}' for key, item in value.items()) + ' }'
    return json.dumps(value, allow_nan=False)


def run_card(case):
    evaluators, rate, regime = case
    sampler = dict(kind='naive_monte_carlo', seed=1234)
    if regime == 'training_burst':
        sampler.update(training_window_samples=max(64, min(100000, math.ceil(rate * .25))),
                       update_timing=dict(overhead_seconds=.5))
    return f'''name = "queue-benchmark"
[evaluator]
kind = "unit"
continuous_dims = 6
timing = {{ per_sample_seconds = {evaluators / rate}, overhead_seconds = 0.001 }}
[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 100
performance_snapshot_interval_ms = 100
[sampler_aggregator_runner_params.queue]
target_batch_eval_ms = 100.0
[evaluator_runner_params]
performance_snapshot_interval_ms = 100
[[task_queue]]
name = "benchmark"
kind = "sample"
stop_condition = {{ max_samples = {rate * MAX_SECONDS * 2} }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {inline(sampler)} }}
'''


def command(args, **kwargs):
    return subprocess.check_output([str(arg) for arg in args], text=True,
                                   stderr=subprocess.STDOUT, timeout=30, **kwargs)


def wait_for(probe):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if probe():
            return
        time.sleep(.1)
    raise TimeoutError('benchmark worker/deployment readiness timed out')


class Deployment:
    def __init__(self, binary, offset):
        self.binary, self.offset = binary, offset
        self.url = f'postgresql://postgres@127.0.0.1:{5400 + offset}/gammaboard_db'
        self.directory = None
        self.process = None
        self.workers = []

    def cli(self, *args):
        return command([self.binary, '--runtime-config', self.runtime,
                        '--port-offset', self.offset, '--json', *args], cwd=ROOT)

    def sql(self, query):
        return command(['psql', self.url, '-XAt', '-v', 'ON_ERROR_STOP=1'], input=query).strip()

    def __enter__(self):
        for port in (8080 + self.offset, 4000 + self.offset, 5400 + self.offset):
            with socket.socket() as sock:
                if sock.connect_ex(('127.0.0.1', port)) == 0:
                    raise RuntimeError(f'port {port} is occupied; choose another --port-offset')
        # Short, isolated paths also avoid PostgreSQL's Unix-socket path limit.
        self.directory = Path(tempfile.mkdtemp(prefix='gmb-bench-', dir='/tmp'))
        self.runtime = self.directory / 'runtime.toml'
        self.runtime.write_text(f'''[resources]
roots = [{inline(str(self.directory / 'resources'))}]
[local_postgres]
socket_dir = {inline(str(self.directory / 'socket'))}
max_connections = 512
''')
        try:
            with (self.directory / 'deploy.log').open('w') as log:
                self.process = subprocess.Popen([str(self.binary), '--runtime-config', str(self.runtime),
                    '--port-offset', str(self.offset), 'deploy'], cwd=ROOT,
                    stdout=log, stderr=subprocess.STDOUT)
            def ready():
                if self.process.poll() is not None:
                    raise RuntimeError('benchmark deployment exited during startup')
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{4000+self.offset}/api/health', timeout=1) as response:
                        return json.load(response).get('database') == 'connected'
                except (OSError, ValueError):
                    return False
            wait_for(ready)
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def __exit__(self, exc_type, *_):
        # The normal deploy supervisor already drains/stops workers and PostgreSQL.
        # Do not duplicate that shutdown protocol here, or delete a live database.
        signal.setitimer(signal.ITIMER_REAL, 0)
        try:
            if self.process is not None:
                if self.process.poll() is None:
                    self.process.terminate()
                if self.process.wait(timeout=30) != 0:
                    raise RuntimeError('benchmark deployment did not shut down cleanly')
            if exc_type is None:
                shutil.rmtree(self.directory)
            else:
                print(f'Diagnostics retained in {self.directory}', file=sys.stderr)
        except BaseException:
            print(f'Cleanup incomplete; inspect {self.directory}', file=sys.stderr)
            raise

    def ensure_workers(self, evaluators):
        if len(self.workers) != evaluators + 1:
            if self.workers:
                self.cli('node', 'stop', *self.workers)
                wait_for(lambda: self.sql('select count(*) from nodes where lease_expires_at>now()') == '0')
            self.workers = json.loads(self.cli('node', 'start-local', evaluators + 1))['node_names']
        wait_for(lambda: self.sql('select count(*) from nodes where lease_expires_at>now()') == str(evaluators + 1))

    def observe(self, run_id):
        started = time.monotonic()
        row = json.loads(self.sql(f'''select json_build_object(
            'samples',t.nr_completed_samples, 'state',t.state,
            'evaluators',(select count(*) from nodes where active_run_id={run_id} and active_role='evaluator' and lease_expires_at>now()),
            'busy',s.runtime_metrics->'avg_evaluator_utilization',
            'updates',coalesce((s.engine_diagnostics->>'training_updates')::bigint,0),
            'update_seconds',coalesce((s.engine_diagnostics->'update_timing'->>'actual_seconds')::float8,0))
            from run_tasks t left join sampler_aggregator_performance_latest s on s.run_id=t.run_id
            where t.run_id={run_id} and t.task->>'kind'='sample';'''))
        row['time'] = (started + time.monotonic()) / 2
        return row

    def measure(self, case, duration):
        evaluators, _, regime = case
        self.ensure_workers(evaluators)
        card = self.directory / 'run.toml'
        card.write_text(run_card(case))
        run_id = json.loads(self.cli('run', 'create', card))['run_id']
        self.cli('run', 'resume', run_id, '--max-evaluators', evaluators)
        wait_for(lambda: self.sql(f'select count(*) from nodes where active_run_id={run_id} and lease_expires_at>now()') == str(evaluators + 1))
        time.sleep(WARMUP_SECONDS)
        rows = [self.observe(run_id)]
        while rows[-1]['time'] - rows[0]['time'] < duration:
            time.sleep(.1)
            rows.append(self.observe(run_id))
        if any(row['state'] != 'active' or row['evaluators'] != evaluators for row in rows):
            raise RuntimeError('benchmark task or worker fleet changed during measurement')
        result = summarize(rows)
        if result['rate'] <= 0 or (regime == 'training_burst' and result['updates'] < 2):
            raise RuntimeError('insufficient progress or fewer than two training stalls; increase --duration')
        self.cli('run', 'pause', run_id)
        wait_for(lambda: self.sql(f'select count(*) from nodes where active_run_id={run_id} and lease_expires_at>now()') == '0')
        self.cli('run', 'remove', '--yes', run_id)
        return result


def summarize(rows):
    first, last = rows[0], rows[-1]
    updates = last['updates'] - first['updates']
    busy = [row['busy'] for row in rows if row['busy'] is not None]
    return dict(rate=(last['samples'] - first['samples']) / (last['time'] - first['time']),
                busy=statistics.mean(busy) if busy else None, updates=updates,
                stall_ms=1000 * (last['update_seconds'] - first['update_seconds']) / updates if updates else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/gammaboard')
    parser.add_argument('--evaluators', type=int, nargs='+', choices=[1, 4, 16, 64], default=[1, 4, 16, 64])
    parser.add_argument('--rates', type=int, nargs='+', default=[1000, 2000000])
    parser.add_argument('--regimes', nargs='+', choices=['inference', 'training_burst'], default=['inference', 'training_burst'])
    parser.add_argument('--duration', type=float, default=4, help='measurement seconds per case (default: 4)')
    parser.add_argument('--port-offset', type=int, default=30)
    args = parser.parse_args()
    matrix = cases(args.evaluators, args.rates, args.regimes)
    if len(set(matrix)) != len(matrix) or any(rate <= 0 for rate in args.rates):
        parser.error('cases must be unique and rates positive')
    if not math.isfinite(args.duration) or args.duration < 1 or not 1 <= args.port_offset <= 57000:
        parser.error('invalid duration or port offset')
    if len(matrix) * (WARMUP_SECONDS + args.duration) + 30 > MAX_SECONDS:
        parser.error('requested measurements exceed the five-minute budget')
    args.binary = args.binary.resolve()
    if not args.binary.is_file():
        parser.error(f'build the binary first: {args.binary}')
    if not shutil.which('psql'):
        parser.error('psql is required; run inside nix develop')
    started = time.monotonic()
    signal.signal(signal.SIGALRM, budget_exhausted)
    signal.setitimer(signal.ITIMER_REAL, MAX_SECONDS - 30)
    print('Regime           Evals    Nominal/s   Measured/s   Capacity   Busy*  Updates   Stall ms', flush=True)
    try:
        with Deployment(args.binary, args.port_offset) as deployment:
            for case in matrix:
                evaluators, nominal, regime = case
                result = deployment.measure(case, args.duration)
                busy = f"{result['busy']:6.1%}" if result['busy'] is not None else '     -'
                print(f"{regime:16} {evaluators:5} {nominal:12,.0f} {result['rate']:12,.0f} "
                      f"{result['rate']/nominal:9.1%} {busy} {result['updates']:8} {result['stall_ms']:10.1f}", flush=True)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    print(f'Completed {len(matrix)} cases in {time.monotonic()-started:.1f}s, including startup and cleanup.')
    print('* Busy uses dashboard rolling metrics; short measurements are coarse. Nominal capacity excludes queue overhead and training stalls.')


def budget_exhausted(*_):
    raise TimeoutError('five-minute benchmark budget exhausted')


def interrupted(*_):
    raise KeyboardInterrupt()


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, interrupted)
    try:
        main()
    except (RuntimeError, TimeoutError, subprocess.SubprocessError, KeyboardInterrupt) as error:
        print(f'Benchmark stopped: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.output, file=sys.stderr)
        sys.exit(1)
