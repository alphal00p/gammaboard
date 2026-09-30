"""Shared benchmark deployment, run lifecycle, cards and measurement contracts."""
from contextlib import contextmanager
import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import hashlib

ROOT = Path(__file__).resolve().parents[1]


class TrialError(RuntimeError):
    """A failed trial whose run was successfully drained and removed."""


@contextmanager
def active_run(session, card, workers, max_age, readiness_seconds=60):
    """All experiments drain their pipeline before direct work or the next case."""
    run = session.cli('run', 'create', card)['run_id']
    try:
        session.cli('run', 'resume', run, '--max-evaluators', workers)
        session.cli('run', 'wait', run, '--until', 'ready', '--evaluators', workers,
                    '--max-age', max_age, '--timeout', f'{readiness_seconds}s', timeout=readiness_seconds + 10)
        yield run
    except Exception as exc:
        try:
            inspection = session.cli('run', 'inspect', run)
        except Exception as diagnostic_error:
            inspection = dict(error=str(diagnostic_error))
        write_json(card.parent/'run-failure.json', inspection)
        if isinstance(exc, (RuntimeError, subprocess.TimeoutExpired)):
            raise TrialError(str(exc)) from exc
        raise
    finally:
        session.cli('run', 'pause', run)
        session.cli('run', 'wait', run, '--until', 'idle')
        session.cli('run', 'remove', '--yes', run)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def file_hash(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def physical_cpus():
    """One logical CPU per physical core, restricted to our existing affinity."""
    allowed = sorted(os.sched_getaffinity(0))
    seen, result = set(), []
    for cpu in allowed:
        base = Path(f'/sys/devices/system/cpu/cpu{cpu}/topology')
        key = (base.joinpath('physical_package_id').read_text().strip(), base.joinpath('core_id').read_text().strip())
        if key not in seen:
            seen.add(key)
            result.append(cpu)
    return result

def idle_cpus(candidates, count):
    def counters():
        result = {}
        for line in Path('/proc/stat').read_text().splitlines():
            label, *values = line.split()
            if label.startswith('cpu') and label[3:].isdigit():
                v = list(map(int, values))
                result[int(label[3:])] = (sum(v[:8]), v[3]+v[4])
        return result
    before = counters()
    time.sleep(0.25)
    after = counters()
    def load(cpu):
        total = after[cpu][0]-before[cpu][0]
        idle = after[cpu][1]-before[cpu][1]
        return (1-idle/total if total else 1, cpu)
    return sorted(sorted(candidates, key=load)[:count])

class Session:
    def __init__(self, binary, output, budget, offset, max_connections=128, infrastructure_cpus=None):
        self.binary, self.output, self.offset = binary, output, offset
        self.deadline = time.monotonic()+budget-100  # Includes the deploy's 75s drain budget.
        self.directory = Path(tempfile.mkdtemp(prefix='gmb-scale-', dir='/tmp'))
        self.runtime = self.directory/'runtime.toml'
        self.runtime.write_text(f'[resources]\nroots = [{json.dumps(str(self.directory/"resources"))}]\n'
                                f'[local_postgres]\nsocket_dir = {json.dumps(str(self.directory/"socket"))}\nmax_connections = {max_connections}\n')
        self.process = None
        self.infrastructure_cpus = infrastructure_cpus
        self.workers = []

    def argv(self, *args):
        return [str(self.binary), '--runtime-config', str(self.runtime), '--port-offset', str(self.offset), '--json', *map(str, args)]

    def cli(self, *args, timeout=90, cpus=None):
        remaining = self.deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('suite time budget exhausted; partial results were retained')
        result = subprocess.run(pinned_command(cpus or self.infrastructure_cpus, self.argv(*args)), cwd=ROOT, capture_output=True, text=True,
                                timeout=min(timeout, remaining))
        if result.returncode:
            raise RuntimeError(f'GammaBoard {args}: {result.stdout}\n{result.stderr}')
        return json.loads(result.stdout)

    def __enter__(self):
        try:
            for port in (8080+self.offset, 4000+self.offset, 5400+self.offset):
                with socket.socket() as sock:
                    if sock.connect_ex(('127.0.0.1', port)) == 0:
                        raise RuntimeError(f'port {port} is occupied; choose another --port-offset')
            with (self.output/'deploy.log').open('w') as log:
                self.process = subprocess.Popen(pinned_command(self.infrastructure_cpus, self.argv('deploy')), cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            deadline = min(self.deadline, time.monotonic()+60)
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    raise RuntimeError('deployment exited; inspect deploy.log')
                try:
                    self.cli('node', 'list', timeout=3)
                    return self
                except (RuntimeError, subprocess.TimeoutExpired):
                    time.sleep(.25)
            raise TimeoutError('deployment readiness timed out')
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def __exit__(self, exc_type, *_):
        errors = []
        started = time.monotonic()
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=90)
            except subprocess.TimeoutExpired:
                errors.append('deployment did not exit within 90s')
        if self.process is not None and self.process.returncode not in (0, None):
            errors.append(f'deployment exited with {self.process.returncode}')
        # One shared deadline: never wait five seconds separately for 512 workers.
        deadline = time.monotonic()+5
        for worker in self.workers:
            try:
                worker.wait(timeout=max(.01, deadline-time.monotonic()))
            except subprocess.TimeoutExpired:
                errors.append('workers remained after deployment shutdown')
                break
        if any(worker.returncode not in (0, None) for worker in self.workers):
            errors.append('a worker exited unsuccessfully')
        if errors:
            # These process groups and this database belong exclusively to the
            # private benchmark. Retain data/logs even when forced cleanup is needed.
            owned = [p for p in [self.process, *self.workers] if p is not None]
            for sig in (signal.SIGTERM, signal.SIGKILL):
                for process in owned:
                    try:
                        os.killpg(process.pid, sig)
                    except ProcessLookupError:
                        pass
                if sig == signal.SIGTERM:
                    time.sleep(1)
            deadline = time.monotonic()+5
            for process in owned:
                try:
                    process.wait(timeout=max(0, deadline-time.monotonic()))
                except subprocess.TimeoutExpired:
                    errors.append(f'owned process {process.pid} did not exit after SIGKILL')
            database = self.directory/'resources/db'/f'postgres-{self.offset}'
            if (database/'postmaster.pid').exists():
                try:
                    subprocess.run(['pg_ctl','-D',str(database),'-m','immediate','stop','-t','10'],
                                   capture_output=True,check=True,timeout=15)
                except (OSError, subprocess.SubprocessError) as error:
                    errors.append(f'private database cleanup failed: {error}')
        write_json(self.output/'cleanup.json', dict(clean=not errors, errors=errors,
                   elapsed_seconds=time.monotonic()-started, runtime_directory=str(self.directory)))
        if exc_type is None and not errors:
            shutil.rmtree(self.directory)
        else:
            print(f'Deployment diagnostics retained in {self.directory}', file=sys.stderr)
            if errors:
                print('; '.join(errors), file=sys.stderr)
                if exc_type is None:
                    raise RuntimeError('; '.join(errors))

    def start_pinned_workers(self, evaluator_cpus, sampler_cpu):
        """Pin each evaluator to its supplied CPU; sampler/database cores are separate."""
        nodes = [('bench-s', sampler_cpu, 'bench_sampler')]
        nodes += [(f'bench-e-{index:03d}', cpu, 'bench_evaluator') for index, cpu in enumerate(evaluator_cpus)]
        for name, cpu, capability in nodes:
            with (self.output / f'{name}.log').open('w') as log:
                self.workers.append(subprocess.Popen(
                    pinned_command([cpu], self.argv('node', 'run', '--name', name, '--capability', f'{capability}=1')),
                    cwd=ROOT, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True))
        deadline = min(self.deadline, time.monotonic() + 60)
        while time.monotonic() < deadline:
            if any(worker.poll() is not None for worker in self.workers):
                raise RuntimeError('a pinned worker exited; inspect its log')
            if len(self.cli('node', 'list')) == len(nodes):
                return
            time.sleep(.25)
        raise TimeoutError('pinned worker registration timed out')

def pinned_command(cpus, command):
    return ['taskset', '-c', ','.join(map(str, cpus)), *command] if cpus else command

def workload(iterations):
    return f'[evaluator]\nkind = "unit"\ncontinuous_dims = 6\ncpu_iterations_per_sample = {iterations}\n'


def calibrate_workloads(session, costs, saved=None, cpus=None):
    values = json.loads(saved.read_text()) if saved else {
        str(cost): session.cli('benchmark', 'calibrate', '--eval-us', cost, cpus=cpus) for cost in costs}
    normalized = {str(float(key)): value for key, value in values.items()}
    if len(normalized) != len(values):
        raise ValueError('duplicate numeric calibration targets')
    for cost in costs:
        calibration = normalized.get(str(float(cost)), {})
        iterations, seconds = calibration.get('cpu_iterations_per_sample'), calibration.get('measured_seconds_per_sample', 0)
        if type(iterations) is not int or iterations <= 0 or not math.isfinite(seconds) or seconds <= 0:
            raise ValueError(f'missing or invalid calibration for {cost}us')
    return normalized


def run_card(iterations, batch_size, min_tick_time_ms=10, telemetry_interval_ms=250,
             generation_batch_size=None):
    return 'name = "scaling-benchmark"\n'+workload(iterations)+f'''
[evaluator_runner_params]
min_tick_time_ms = {min_tick_time_ms}
performance_snapshot_interval_ms = {telemetry_interval_ms}
[sampler_aggregator_runner_params]
min_tick_time_ms = {min_tick_time_ms}
frontend_sync_interval_ms = {telemetry_interval_ms}
performance_snapshot_interval_ms = {telemetry_interval_ms}
[sampler_aggregator_runner_params.queue]
fixed_batch_size = {batch_size}
max_batch_size = {batch_size}
[[task_queue]]
name = "measure"
kind = "sample"
stop_condition = {{ max_samples = 1000000000000 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 1234, generation_batch_size = {generation_batch_size or 1048576} }} }}
'''

def load_results(directory):
    path = directory/'results.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()] if path.exists() else []


def preserve_inputs(output, binary):
    """A later build or source edit must not change an experiment already running."""
    copied = output / 'gammaboard'
    shutil.copy2(binary, copied)
    harness = output / 'harness'
    harness.mkdir()
    for source in (ROOT/'scripts').glob('benchmark*.py'):
        shutil.copy2(source, harness/source.name)
    migrations = Path(os.environ.get('GAMMABOARD_MIGRATIONS_DIR', ROOT/'migrations'))
    shutil.copytree(migrations, output/'migrations')
    os.environ['GAMMABOARD_MIGRATIONS_DIR'] = str(output/'migrations')
    return copied, {p.name: file_hash(p) for p in harness.glob('*.py')}


def activity(measurement):
    first, last = measurement['snapshots'][0], measurement['snapshots'][-1]
    result = {}
    for collection, field, role in [('evaluators','metrics','evaluator'),('samplers','runtime_metrics','sampler')]:
        start = {r['worker_id']: r[field] for r in first[collection]}
        end_ids = [r['worker_id'] for r in last[collection]]
        if not start or set(start) != set(end_ids) or len(start) != len(first[collection]) or len(set(end_ids)) != len(end_ids):
            raise ValueError('missing, duplicate or changed worker coverage')
        total = dict(elapsed_seconds=0., compute_seconds=0., io_seconds=0.)
        for row in last[collection]:
            a, b = start[row['worker_id']], row[field]
            epoch = 'epoch' if role == 'evaluator' else 'runner_epoch'
            if any(a.get(k) is None or a.get(k) != b.get(k) for k in [epoch,'node_uuid','task_id']):
                raise ValueError('worker identity changed')
            delta = {k: b['busy'][k]-a['busy'][k] for k in total}
            elapsed = delta['elapsed_seconds']
            if elapsed <= 0 or any(not math.isfinite(v) or v < 0 or v > elapsed+1e-9 for v in delta.values()):
                raise ValueError('invalid busy interval')
            for k, v in delta.items(): total[k] += v
        for lane in ['compute','io']:
            result[f'{role}_{lane}'] = 100*total[f'{lane}_seconds']/total['elapsed_seconds']
    return result

def payload_throughput(measurement, batch_size):
    """Accepted fixed-batch input volume, not wire traffic or physical disk I/O."""
    windows = {}
    for snapshot in measurement['snapshots']:
        for row in snapshot['samplers']:
            metric = row['runtime_metrics']['queue']['rolling']['insert_bundle_payload_bytes_per_batch']
            if metric['count']:
                windows[row['worker_id'],row['id']] = metric
    batches_per_second = measurement['samples_per_second']/batch_size
    values = list(windows.values())
    # Fixed sample count, dimension and accumulator must yield a constant encoded
    # payload. A mixed/startup window cannot support this conversion reliably.
    if not values or any(v['mean'] is None or not math.isfinite(v['mean']) or v['mean'] <= 0
                         or not math.isfinite(v['std_dev']) or v['std_dev'] > 1e-6 for v in values):
        raise ValueError('missing or variable fixed-batch payload size')
    sizes = [v['mean'] for v in values]
    if max(sizes)-min(sizes) > 1e-6:
        raise ValueError('fixed-batch payload size changed during measurement')
    payload = statistics.mean(sizes)
    return dict(batches_per_second=batches_per_second,input_payload_bytes_per_batch=payload,
                accepted_input_mib_per_second=batches_per_second*payload/1024**2)
