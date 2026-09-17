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
        if not isinstance(s.get(key), (float, int)) or not math.isfinite(s[key]) or s[key] <= 0:
            raise ValueError(f'{key} must be finite and positive')
    if s['duration_seconds'] < 4*s.get('telemetry_interval_ms',250)/1000:
        raise ValueError('duration must cover at least four telemetry publication intervals')
    if s['budget_seconds'] > 1800:
        raise ValueError('suite budget must be at most 1800 seconds')
    if type(s.get('seed',1234)) is not int:
        raise ValueError('seed must be an integer')
    if type(s.get('min_tick_time_ms',0)) is not int or s.get('min_tick_time_ms',0) < 0:
        raise ValueError('min_tick_time_ms must be a nonnegative integer')
    if type(s.get('telemetry_interval_ms',250)) is not int or not 100 <= s.get('telemetry_interval_ms',250) <= 5000:
        raise ValueError('telemetry_interval_ms must be between 100 and 5000')
    if max(s['workers']) > s['cpu_limit']:
        raise ValueError('worker count exceeds the CPU budget')
    # Long batches make bounded measurements and shutdown uninformative.
    if max(s['eval_us'])*max(s['batch_sizes'])/1e6 > s['duration_seconds']/2:
        raise ValueError('largest batch exceeds half the measurement duration')
    cases = list(itertools.product(s['eval_us'], s['workers'], s['batch_sizes'], range(s['repetitions'])))
    estimate = len(cases)*(3*(s['duration_seconds']+s['warmup_seconds'])+5)+45
    if estimate > s['budget_seconds']:
        raise ValueError(f'planned measurements need about {estimate:.0f}s; reduce the matrix or durations')
    return cases


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
    def __init__(self, binary, output, budget, offset):
        self.binary, self.output, self.offset = binary, output, offset
        self.deadline = time.monotonic()+budget-30  # Reserve cleanup time.
        self.directory = Path(tempfile.mkdtemp(prefix='gmb-scale-', dir='/tmp'))
        self.runtime = self.directory/'runtime.toml'
        self.runtime.write_text(f'[resources]\nroots = [{json.dumps(str(self.directory/"resources"))}]\n'
                                f'[local_postgres]\nsocket_dir = {json.dumps(str(self.directory/"socket"))}\nmax_connections = 128\n')
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


def run_card(iterations, batch_size, min_tick_time_ms=0, telemetry_interval_ms=250):
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
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 1234 }} }}
'''


def execute(args):
    suite = tomllib.loads(args.suite.read_text())
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
    random.Random(suite.get('seed',1234)).shuffle(cases)
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
                evaluator_card = directory/'evaluator.toml'
                evaluator_card.write_text(workload(iterations))
                card = directory/'run.toml'
                card.write_text(run_card(iterations,batch_size,suite.get("min_tick_time_ms",0),suite.get("telemetry_interval_ms",250)))
                direct = {}
                for n in sorted({1,workers}):
                    direct[n] = session.cli('benchmark','evaluator',evaluator_card,'--workers',n,'--batch-size',batch_size,
                                            '--warmup',f'{suite["warmup_seconds"]}s','--duration',f'{suite["duration_seconds"]}s')
                    write_json(directory/f'direct-{n}.json',direct[n])
                run_id = session.cli('run','create',card)['run_id']
                session.cli('run','resume',run_id,'--max-evaluators',workers)
                session.cli('run','wait',run_id,'--until','ready','--evaluators',workers)
                # Warmup is observed as a discarded interval, never a Python readiness sleep.
                session.cli('run','performance',run_id,'--duration',f'{suite["warmup_seconds"]}s')
                measurement = session.cli('run','performance',run_id,'--duration',f'{suite["duration_seconds"]}s')
                write_json(directory/'gammaboard.json',measurement)
                session.cli('run','pause',run_id)
                session.cli('run','wait',run_id,'--until','idle')
                session.cli('run','remove','--yes',run_id)
                rate = measurement['samples_per_second']
                valid = measurement['valid'] and rate is not None and rate > 0
                issues = list(measurement['issues'])
                if measurement['valid'] and (rate is None or rate <= 0):
                    issues.append('no accepted samples during the measurement interval')
                record = dict(schema_version=1,case=index,eval_us=cost,workers=workers,batch_size=batch_size,repeat=repeat,
                              cpu_iterations_per_sample=iterations,valid=valid,issues=issues,
                              rate=rate if valid else None,direct_serial_rate=direct[1]['samples_per_second'],
                              direct_parallel_rate=direct[workers]['samples_per_second'],
                              speedup=rate/direct[1]['samples_per_second'] if valid else None,
                              retained_efficiency=rate/direct[workers]['samples_per_second'] if valid else None)
                records.append(record)
                with (output/'results.jsonl').open('a') as f:
                    f.write(json.dumps(record,allow_nan=False)+'\n')
                print(f'{index+1}/{len(cases)}: {cost}us, {workers} evaluators, batch {batch_size}: '
                      +(f'{rate:,.0f}/s, {record["retained_efficiency"]:.1%} retained' if valid else f'INVALID {record["issues"]}'),flush=True)
        metadata['invalid_cases'] = sum(not row['valid'] for row in records)
        metadata['status'] = 'completed' if not metadata['invalid_cases'] else 'completed_with_invalid_cases'
    except BaseException as exc:
        metadata.update(status='incomplete',error=str(exc))
        raise
    finally:
        metadata.update(completed_cases=len(records), finished_at=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
        write_json(output/'manifest.json',metadata)
    if metadata.get('invalid_cases'):
        raise RuntimeError(f"{metadata['invalid_cases']} invalid cases; inspect saved issues before plotting")
    return output


def load_results(directory):
    return [json.loads(line) for line in (directory/'results.jsonl').read_text().splitlines() if line.strip()]


def grouped(records, metric):
    groups = {}
    for row in records:
        if row['valid']:
            groups.setdefault((row['batch_size'],row['workers'],row['eval_us']),[]).append(row[metric])
    return groups


def plot(directory):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    records = load_results(directory)
    manifest_path = directory/'manifest.json'
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
    context = f"{manifest.get('physical_core_budget','?')}-core cap; tick floor {manifest.get('suite',{}).get('min_tick_time_ms',0)}ms"
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
                if math.isfinite(value): ax.text(j,i,f'{value:.0%}',ha='center',va='center',color='white' if value<.6 else 'black')
        ax.set_title(f'Retained parallel throughput — batch {batch}\n{context}')
        fig.colorbar(heat,ax=ax,label='GammaBoard / direct parallel'); fig.tight_layout()
        for extension in ('png','svg'): fig.savefig(directory/f'efficiency-map-batch-{batch}.{extension}')
        plt.close(fig)
    summary = []
    for batch,workers in sorted({key[:2] for key in groups}):
        costs = sorted(key[2] for key in groups if key[:2]==(batch,workers))
        summary.append(dict(batch_size=batch,workers=workers,
            minimum_tested_eval_us_at_80_percent=next((c for c in costs if statistics.median(groups[batch,workers,c])>=.8),None),
            minimum_tested_eval_us_at_90_percent=next((c for c in costs if statistics.median(groups[batch,workers,c])>=.9),None)))
    write_json(directory/'thresholds.json',dict(schema_version=1,criterion='median retained parallel throughput; tested points only, not an interpolated crossover',rows=summary))
    return directory


def compare(before, after):
    left, right = load_results(before), load_results(after)
    left_work = {(r['batch_size'],r['workers'],r['eval_us']):r['cpu_iterations_per_sample'] for r in left}
    right_work = {(r['batch_size'],r['workers'],r['eval_us']):r['cpu_iterations_per_sample'] for r in right}
    if any(left_work[k] != right_work[k] for k in left_work.keys() & right_work.keys()):
        raise ValueError('calibrated work differs; rerun with --calibration pointing to the original calibration.json')
    a, b = grouped(left,'rate'), grouped(right,'rate')
    print('Batch  Workers  Eval µs  Before/s  After/s  Ratio')
    for key in sorted(a.keys() & b.keys()):
        x,y = statistics.median(a[key]),statistics.median(b[key])
        print(f'{key[0]:5} {key[1]:8} {key[2]:8} {x:9.1f} {y:8.1f} {y/x:6.3f}')
    print('Compare calibrated work counts and machine metadata before attributing changes to code.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command',required=True)
    run = commands.add_parser('run')
    run.add_argument('suite',type=Path)
    run.add_argument('--binary',type=Path,default=ROOT/'target/release/gammaboard')
    run.add_argument('--output',type=Path,required=True)
    run.add_argument('--port-offset',type=int,default=50)
    run.add_argument('--calibration',type=Path,help='reuse fixed work from a previous calibration.json for revision comparisons')
    graph = commands.add_parser('plot'); graph.add_argument('directory',type=Path)
    diff = commands.add_parser('compare'); diff.add_argument('before',type=Path); diff.add_argument('after',type=Path)
    args = parser.parse_args()
    try:
        if args.command=='run':
            if not 1 <= args.port_offset <= 57000: raise ValueError('invalid port offset')
            print(execute(args))
        elif args.command=='plot': print(plot(args.directory))
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
