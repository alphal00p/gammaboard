#!/usr/bin/env python3
"""CLI-only GammaBoard experiments. Python 3.11+; matplotlib is needed only for plots."""
import argparse
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import random
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def validate_suite(s):
    allowed = {'eval_us', 'workers', 'batch_sizes', 'repetitions', 'cpu_limit',
               'duration_seconds', 'warmup_seconds', 'budget_seconds', 'seed',
               'min_tick_time_ms', 'telemetry_interval_ms', 'bulk_sample_generation',
               'generation_batch_size'}
    if unknown := s.keys() - allowed:
        raise ValueError(f'unknown suite settings: {sorted(unknown)}')
    for key in ('eval_us', 'workers', 'batch_sizes'):
        values = s.get(key)
        if not isinstance(values, list) or not values or any(type(v) is not int or v <= 0 for v in values) or len(set(values)) != len(values):
            raise ValueError(f'{key} must contain unique positive integers')
    if min(s['eval_us']) < 1 or max(s['eval_us']) > 100000:
        raise ValueError('eval_us must be between 1 and 100000')
    if min(s['batch_sizes']) < 16 or max(s['batch_sizes']) > 1000000:
        raise ValueError('batch_sizes must be between 16 and 1000000')
    for key in ('repetitions', 'cpu_limit'):
        if type(s.get(key)) is not int or s[key] < 1:
            raise ValueError(f'{key} must be a positive integer')
    for key in ('duration_seconds', 'warmup_seconds', 'budget_seconds'):
        if type(s.get(key)) not in (float, int) or not math.isfinite(s[key]) or s[key] <= 0:
            raise ValueError(f'{key} must be finite and positive')
    if s['budget_seconds'] > 1800:
        raise ValueError('suite budget must be at most 1800 seconds')
    if type(s.get('seed',1234)) is not int:
        raise ValueError('seed must be an integer')
    if type(s.get('min_tick_time_ms',10)) is not int or s.get('min_tick_time_ms',10) < 0:
        raise ValueError('min_tick_time_ms must be a nonnegative integer')
    if type(s.get('telemetry_interval_ms',250)) is not int or not 100 <= s.get('telemetry_interval_ms',250) <= 5000:
        raise ValueError('telemetry_interval_ms must be between 100 and 5000')
    if s['duration_seconds'] < 8*s.get('telemetry_interval_ms',250)/1000:
        raise ValueError('duration must cover at least eight telemetry publication intervals')
    if s['warmup_seconds'] < 2*s.get('telemetry_interval_ms',250)/1000:
        raise ValueError('warmup must cover at least two telemetry publication intervals')
    if type(s.get('bulk_sample_generation', False)) is not bool:
        raise ValueError('bulk_sample_generation must be boolean')
    generation_size = s.get('generation_batch_size', max(s['batch_sizes']))
    if type(generation_size) is not int or not max(s['batch_sizes']) <= generation_size <= 1000000:
        raise ValueError('generation_batch_size must cover all evaluator batches and be at most 1000000')
    if 'generation_batch_size' in s and not s.get('bulk_sample_generation', False):
        raise ValueError('generation_batch_size requires bulk_sample_generation')
    if max(s['workers']) > s['cpu_limit']:
        raise ValueError('worker count exceeds the CPU budget')
    # Long batches make bounded measurements and shutdown uninformative.
    if max(s['eval_us'])*max(s['batch_sizes'])/1e6 > s['duration_seconds']/2:
        raise ValueError('largest batch exceeds half the measurement duration')
    estimate = estimated_seconds(s)
    if estimate > s['budget_seconds']:
        raise ValueError(f'planned measurements need about {estimate:.0f}s; reduce the matrix or durations')
    return list(itertools.product(s['eval_us'], s['workers'], s['batch_sizes'], range(s['repetitions'])))


def estimated_seconds(suite):
    # Serial and parallel are the same measurement at one evaluator.
    configurations = len(suite['eval_us']) * len(suite['batch_sizes']) * suite['repetitions']
    phases = configurations * sum(2 if workers == 1 else 3 for workers in suite['workers'])
    return phases * (suite['duration_seconds'] + suite['warmup_seconds']) + 5 * configurations * len(suite['workers']) + 60


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
    def __init__(self, binary, output, budget, offset, max_connections=128):
        self.binary, self.output, self.offset = binary, output, offset
        self.deadline = time.monotonic()+budget-30  # Reserve cleanup time.
        self.directory = Path(tempfile.mkdtemp(prefix='gmb-scale-', dir='/tmp'))
        self.runtime = self.directory/'runtime.toml'
        self.runtime.write_text(f'[resources]\nroots = [{json.dumps(str(self.directory/"resources"))}]\n'
                                f'[local_postgres]\nsocket_dir = {json.dumps(str(self.directory/"socket"))}\nmax_connections = {max_connections}\n')
        self.process = None

    def argv(self, *args):
        return [str(self.binary), '--runtime-config', str(self.runtime), '--port-offset', str(self.offset), '--json', *map(str, args)]

    def cli(self, *args, timeout=90):
        remaining = self.deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('suite time budget exhausted; partial results were retained')
        result = subprocess.run(self.argv(*args), cwd=ROOT, capture_output=True, text=True,
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
                self.process = subprocess.Popen(self.argv('deploy'), cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
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
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                print(f'Deployment still running (pid={self.process.pid}); retained {self.directory}', file=sys.stderr)
                raise
        if exc_type is None and (self.process is None or self.process.returncode == 0):
            shutil.rmtree(self.directory)
        else:
            print(f'Deployment diagnostics retained in {self.directory}', file=sys.stderr)
            if exc_type is None:
                raise RuntimeError(f'deployment did not shut down cleanly (exit {self.process.returncode})')


def workload(iterations):
    return f'[evaluator]\nkind = "unit"\ncontinuous_dims = 6\ncpu_iterations_per_sample = {iterations}\n'


def run_card(iterations, batch_size, min_tick_time_ms=10, telemetry_interval_ms=250,
             bulk_sample_generation=False, generation_batch_size=None):
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
max_batch_size = {generation_batch_size or batch_size}
bulk_sample_generation = {str(bulk_sample_generation).lower()}
[[task_queue]]
name = "measure"
kind = "sample"
stop_condition = {{ max_samples = 1000000000000 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 1234 }} }}
'''


def measure_case(session, directory, suite, cost, workers, batch_size, repeat, iterations):
    evaluator_card = directory/'evaluator.toml'
    evaluator_card.write_text(workload(iterations))
    card = directory/'run.toml'
    card.write_text(run_card(iterations, batch_size, suite.get('min_tick_time_ms', 10),
                             suite.get('telemetry_interval_ms', 250),
                             suite.get('bulk_sample_generation', False),
                             suite.get('generation_batch_size')))
    direct, measurement = {}, None
    # Rotate the pipeline's position between repetitions to avoid always measuring
    # it last, after both direct baselines have heated the same cores.
    order = [f'direct-{n}' for n in sorted({1, workers})]
    order.insert(repeat % (len(order) + 1), 'gammaboard')
    interval = f'{suite.get("telemetry_interval_ms", 250)}ms'
    max_age = f'{max(1000, 4 * suite.get("telemetry_interval_ms", 250))}ms'
    for backend in order:
        if backend != 'gammaboard':
            n = int(backend.split('-')[1])
            direct[n] = session.cli('benchmark', 'evaluator', evaluator_card,
                                    '--workers', n, '--batch-size', batch_size,
                                    '--warmup', f'{suite["warmup_seconds"]}s',
                                    '--duration', f'{suite["duration_seconds"]}s')
            write_json(directory/f'{backend}.json', direct[n])
            continue
        run_id = session.cli('run', 'create', card)['run_id']
        session.cli('run', 'resume', run_id, '--max-evaluators', workers)
        session.cli('run', 'wait', run_id, '--until', 'ready', '--evaluators', workers,
                    '--max-age', max_age)
        # Readiness and discarded warmup are distinct. Save both the warmup and
        # measurement so startup effects can be inspected without repeating work.
        for name, duration in [('warmup', suite['warmup_seconds']),
                               ('gammaboard', suite['duration_seconds'])]:
            observation = session.cli('run', 'performance', run_id, '--duration', f'{duration}s',
                                       '--interval', interval, '--max-age', max_age)
            write_json(directory/f'{name}.json', observation)
            if name == 'gammaboard':
                measurement = observation
        session.cli('run', 'pause', run_id)
        session.cli('run', 'wait', run_id, '--until', 'idle')
        session.cli('run', 'remove', '--yes', run_id)
    return measurement_record(measurement, direct, eval_us=cost, workers=workers,
                              batch_size=batch_size, repeat=repeat,
                              cpu_iterations_per_sample=iterations, measurement_order=order)


def measurement_record(measurement, direct, **case):
    rate = measurement['samples_per_second']
    issues = list(measurement['issues'])
    if not measurement['valid'] and not issues:
        issues.append('invalid GammaBoard interval')
    for label, value in [('GammaBoard', rate)] + [
            (f'direct-{n}', result['samples_per_second']) for n, result in direct.items()]:
        if value is None or not math.isfinite(value) or value <= 0:
            issues.append(f'{label}: no finite positive throughput')
    valid = not issues
    serial = direct[1]['samples_per_second']
    parallel = direct[case['workers']]['samples_per_second']
    # Keep invalid intervals explicit; they must never become zero-throughput points.
    return dict(schema_version=1, **case, valid=valid, issues=issues,
                rate=rate if valid else None,
                direct_serial_rate=serial if serial is not None and math.isfinite(serial) else None,
                direct_parallel_rate=parallel if parallel is not None and math.isfinite(parallel) else None,
                speedup=rate/serial if valid else None,
                retained_efficiency=rate/parallel if valid else None)


def execute(args):
    suite = tomllib.loads(args.suite.read_text())
    suite.setdefault('min_tick_time_ms', 10)
    suite.setdefault('telemetry_interval_ms', 250)
    suite.setdefault('bulk_sample_generation', False)
    cases = validate_suite(suite)
    if not hasattr(os, 'sched_setaffinity'):
        raise RuntimeError('CPU-bounded runs currently require Linux affinity support')
    candidates = physical_cpus()
    cap = min(8, max(1, len(candidates)//4))
    if suite['cpu_limit'] > cap:
        raise ValueError(f'cpu_limit exceeds the shared-host cap of {cap} physical cores')
    cpus = idle_cpus(candidates, suite['cpu_limit'])
    os.sched_setaffinity(0, cpus)
    os.nice(5)
    for key in ('OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'MKL_NUM_THREADS', 'RAYON_NUM_THREADS', 'TOKIO_WORKER_THREADS'):
        os.environ[key] = '1'
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    shutil.copyfile(args.suite, output/'suite.toml')
    with binary.open('rb') as binary_file:
        binary_hash = hashlib.file_digest(binary_file,'sha256').hexdigest()
    metadata = dict(schema_version=1, suite=suite, cpus=cpus, physical_core_budget=len(cpus),
                    cpu_model=next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), None),
                    load_average=os.getloadavg(), binary=str(binary), binary_sha256=binary_hash,
                    started_at=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                    scope='CPU synthetic, fixed batches, warm inference; direct baseline includes per-worker uniform generation/materialization/scalar accumulation',
                    status='running')
    write_json(output/'manifest.json', metadata)
    # Complete one repetition of the whole matrix before starting the next one.
    # An interrupted development run then covers more configurations.
    rng = random.Random(suite.get('seed',1234))
    cases = [case for repeat in range(suite['repetitions'])
             for case in rng.sample([c for c in cases if c[3] == repeat],
                                     len(cases)//suite['repetitions'])]
    records = []
    try:
        with Session(binary, output, suite['budget_seconds'], args.port_offset) as session:
            if args.calibration:
                calibrations = json.loads(args.calibration.read_text())
                for cost in suite['eval_us']:
                    iterations = calibrations[str(cost)]['cpu_iterations_per_sample']
                    if type(iterations) is not int or iterations <= 0:
                        raise ValueError('invalid saved calibration')
            else:
                calibrations = {str(cost): session.cli('benchmark','calibrate','--eval-us',cost) for cost in suite['eval_us']}
            write_json(output/'calibration.json', calibrations)
            session.cli('node','start-local',max(suite['workers'])+1)
            for index, (cost, workers, batch_size, repeat) in enumerate(cases):
                directory = output/f'case-{index:03d}'
                directory.mkdir()
                iterations = calibrations[str(cost)]['cpu_iterations_per_sample']
                record = measure_case(session, directory, suite, cost, workers, batch_size, repeat, iterations)
                record['case'] = index
                records.append(record)
                with (output/'results.jsonl').open('a') as f:
                    f.write(json.dumps(record,allow_nan=False)+'\n')
                print(f'{index+1}/{len(cases)}: {cost}us, {workers} evaluators, batch {batch_size}: '
                      +(f'{record["rate"]:,.0f}/s, {record["retained_efficiency"]:.1%} retained' if record['valid'] else f'INVALID {record["issues"]}'),flush=True)
        metadata['invalid_cases'] = sum(not row['valid'] for row in records)
        metadata['status'] = 'completed' if not metadata['invalid_cases'] else 'completed_with_invalid_cases'
    except BaseException as exc:
        metadata.update(status='incomplete',error=str(exc) or type(exc).__name__)
        raise
    finally:
        metadata.update(completed_cases=len(records), finished_at=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
        write_json(output/'manifest.json',metadata)
        write_json(output/'summary.json', summarize(records, suite))
    if metadata.get('invalid_cases'):
        raise RuntimeError(f"{metadata['invalid_cases']} invalid cases; inspect saved issues before plotting")
    return output


def load_results(directory):
    path = directory/'results.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()] if path.exists() else []


def grouped(records, metric):
    groups = {}
    for row in records:
        if row['valid']:
            groups.setdefault((row['batch_size'],row['workers'],row['eval_us']),[]).append(row[metric])
    return groups


def summarize(records, suite):
    groups = {key: [] for key in itertools.product(suite['batch_sizes'], suite['workers'], suite['eval_us'])}
    seen = set()
    for row in records:
        key = row['batch_size'], row['workers'], row['eval_us']
        identity = (*key, row['repeat'])
        if identity in seen or key not in groups or row['repeat'] not in range(suite['repetitions']):
            raise ValueError(f'duplicate or unplanned trial: {identity}')
        seen.add(identity)
        groups[key].append(row)
    rows = []
    for (batch, workers, cost), trials in sorted(groups.items()):
        valid = [row for row in trials if row['valid']]
        metrics = {}
        for metric in ('rate', 'speedup', 'retained_efficiency'):
            values = [row[metric] for row in valid]
            metrics[metric] = dict(median=statistics.median(values), minimum=min(values),
                                   maximum=max(values)) if values else None
        rows.append(dict(batch_size=batch, workers=workers, eval_us=cost,
                         planned_trials=suite['repetitions'], valid_trials=len(valid),
                         invalid_trials=len(trials)-len(valid),
                         missing_trials=suite['repetitions']-len(trials), **metrics))
    return dict(schema_version=1, planned_cases=len(groups)*suite['repetitions'],
                completed_cases=len(records), valid_cases=sum(row['valid'] for row in records),
                complete=all(row['valid_trials'] == suite['repetitions'] for row in rows), rows=rows)


def summary(directory):
    manifest = json.loads((directory/'manifest.json').read_text())
    result = summarize(load_results(directory), manifest['suite'])
    write_json(directory/'summary.json', result)
    print(f'{result["valid_cases"]}/{result["planned_cases"]} valid trials; '
          f'{"complete" if result["complete"] else "INCOMPLETE"}')
    print('Batch  Workers  Eval µs  Valid/Plan  Median/s  Retained (trial range)')
    for row in result['rows']:
        prefix = (f'{row["batch_size"]:5} {row["workers"]:8} {row["eval_us"]:8} '
                  f'{row["valid_trials"]:5}/{row["planned_trials"]:<4}')
        if row['rate'] is None:
            print(f'{prefix}  missing/invalid')
        else:
            efficiency = row['retained_efficiency']
            print(f'{prefix} {row["rate"]["median"]:9.1f}  {efficiency["median"]:.1%} '
                  f'({efficiency["minimum"]:.1%}–{efficiency["maximum"]:.1%})')
    return result


def plot(directory):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    records = load_results(directory)
    manifest = json.loads((directory/'manifest.json').read_text())
    coverage = summarize(records, manifest['suite'])
    context = (f"{manifest['physical_core_budget']}-core cap; "
               f"{coverage['valid_cases']}/{coverage['planned_cases']} valid trials; "
               f"tick {manifest['suite'].get('min_tick_time_ms', 0)}ms")
    if not coverage['complete']:
        context += ' — INCOMPLETE'
    if not any(r['valid'] for r in records):
        raise ValueError('no valid measurements to plot')
    for metric, label in [('retained_efficiency','Throughput / direct parallel throughput'),('speedup','Throughput / direct serial throughput')]:
        groups = grouped(records,metric)
        for batch in sorted({key[0] for key in groups}):
            fig, ax = plt.subplots(figsize=(8,5))
            for workers in sorted({key[1] for key in groups if key[0]==batch}):
                costs = sorted(key[2] for key in groups if key[:2]==(batch,workers))
                values = [groups[batch,workers,c] for c in costs]
                medians = [statistics.median(v) for v in values]
                # Trial range, explicitly not a confidence interval from correlated snapshots.
                ax.errorbar(costs,medians,yerr=[[m-min(v) for m,v in zip(medians,values)],
                                              [max(v)-m for m,v in zip(medians,values)]],marker='o',label=f'{workers} evaluators')
            ax.set_xscale('log'); ax.set_xlabel('Calibration target per sample (µs)')
            ax.set_ylabel(label); ax.axhline(1,color='grey',linestyle='--')
            if metric=='retained_efficiency': ax.axhline(.8,color='grey',linestyle=':')
            ax.set_title(f'Batch {batch} — median and trial range\n{context}'); ax.legend(); fig.tight_layout()
            for extension in ('png','svg'): fig.savefig(directory/f'{metric}-batch-{batch}.{extension}')
            plt.close(fig)
    groups = grouped(records,'retained_efficiency')
    for batch in sorted({key[0] for key in groups}):
        costs = sorted({key[2] for key in groups if key[0]==batch})
        workers = sorted({key[1] for key in groups if key[0]==batch})
        grid = [[statistics.median(groups[batch,n,c]) if (batch,n,c) in groups else float('nan')
                 for c in costs] for n in workers]
        fig, ax = plt.subplots(figsize=(8,4))
        heat = ax.imshow(grid,aspect='auto',origin='lower',vmin=0,vmax=1,cmap='viridis')
        ax.set_xticks(range(len(costs)),labels=costs); ax.set_yticks(range(len(workers)),labels=workers)
        ax.set_xlabel('Calibration target per sample (µs)'); ax.set_ylabel('Evaluator workers')
        for i,row in enumerate(grid):
            for j,value in enumerate(row):
                if math.isfinite(value):
                    n = len(groups[batch, workers[i], costs[j]])
                    ax.text(j,i,f'{value:.0%}\nn={n}',ha='center',va='center',color='white' if value<.6 else 'black')
        ax.set_title(f'Retained parallel throughput — batch {batch}\n{context}')
        fig.colorbar(heat,ax=ax,label='GammaBoard / direct parallel'); fig.tight_layout()
        for extension in ('png','svg'): fig.savefig(directory/f'efficiency-map-batch-{batch}.{extension}')
        plt.close(fig)
    write_json(directory/'summary.json', coverage)
    return directory


def compare(before, after):
    left, right = load_results(before), load_results(after)
    def work_counts(records):
        result = {}
        for row in records:
            key = row['batch_size'], row['workers'], row['eval_us']
            result.setdefault(key, set()).add(row['cpu_iterations_per_sample'])
        if any(len(counts) != 1 for counts in result.values()):
            raise ValueError('calibrated work changes between repetitions within a suite')
        return result
    left_work, right_work = work_counts(left), work_counts(right)
    if any(left_work[k] != right_work[k] for k in left_work.keys() & right_work.keys()):
        raise ValueError('calibrated work differs; rerun with --calibration pointing to the original calibration.json')
    a, b = grouped(left,'rate'), grouped(right,'rate')
    if not a.keys() & b.keys():
        raise ValueError('no overlapping valid cases to compare')
    print('Batch  Workers  Eval µs  Trials A/B  Before/s  After/s  Ratio')
    for key in sorted(a.keys() & b.keys()):
        x,y = statistics.median(a[key]),statistics.median(b[key])
        print(f'{key[0]:5} {key[1]:8} {key[2]:8} {len(a[key]):5}/{len(b[key]):<5} {x:9.1f} {y:8.1f} {y/x:6.3f}')
    print('Compare calibrated work counts and machine metadata before attributing changes to code.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command',required=True)
    plan = commands.add_parser('plan', help='validate a suite and estimate runtime without launching work')
    plan.add_argument('suite', type=Path)
    run = commands.add_parser('run')
    run.add_argument('suite',type=Path)
    run.add_argument('--binary',type=Path,default=ROOT/'target/release/gammaboard')
    run.add_argument('--output',type=Path,required=True)
    run.add_argument('--port-offset',type=int,default=50)
    run.add_argument('--calibration',type=Path,help='reuse fixed work from a previous calibration.json for revision comparisons')
    io = commands.add_parser('io', help='compare 1/2/8 inserts with fast integrands and large fleets')
    io.add_argument('--binary',type=Path,default=ROOT/'target/release/gammaboard')
    io.add_argument('--output',type=Path,required=True)
    io.add_argument('--workers',type=int,nargs='+',default=[1,8,32,64])
    io.add_argument('--batch-sizes',type=int,nargs='+',default=[16,256])
    io.add_argument('--inserts',type=int,nargs='+',default=[1,2,8])
    io.add_argument('--insert-bundle-size',type=int,default=5)
    io.add_argument('--input-storage',choices=['default','pglz','lz4','external'],default='default',
                    help='optional PostgreSQL input-column experiment; requires psql')
    io.add_argument('--repetitions',type=int,default=3)
    io.add_argument('--duration',type=float,default=8)
    io.add_argument('--warmup',type=float,default=2)
    io.add_argument('--iterations',type=int,default=0)
    io.add_argument('--min-tick-ms',type=int,default=1)
    io.add_argument('--cpu-limit',type=int,default=8)
    io.add_argument('--port-offset',type=int,default=70)
    graph = commands.add_parser('plot'); graph.add_argument('directory',type=Path)
    report = commands.add_parser('summary'); report.add_argument('directory', type=Path)
    diff = commands.add_parser('compare'); diff.add_argument('before',type=Path); diff.add_argument('after',type=Path)
    args = parser.parse_args()
    try:
        if args.command=='plan':
            suite = tomllib.loads(args.suite.read_text())
            cases = validate_suite(suite)
            print(f'{len(cases)} trials; estimated {estimated_seconds(suite)/60:.1f} min; '
                  f'hard budget {suite["budget_seconds"]/60:.1f} min; {suite["cpu_limit"]} physical cores')
        elif args.command=='run':
            if not 1 <= args.port_offset <= 57000: raise ValueError('invalid port offset')
            print(execute(args))
        elif args.command=='io':
            import benchmark_io
            print(benchmark_io.execute(args))
        elif args.command=='plot': print(plot(args.directory))
        elif args.command=='summary': summary(args.directory)
        else: compare(args.before,args.after)
    except (ValueError, RuntimeError, TimeoutError, OSError, subprocess.SubprocessError, KeyboardInterrupt) as exc:
        print(f'Benchmark stopped: {exc}',file=sys.stderr)
        return 1
    return 0


def interrupted(*_):
    raise KeyboardInterrupt()


if __name__=='__main__':
    signal.signal(signal.SIGTERM,interrupted)
    sys.exit(main())
