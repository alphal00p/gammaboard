#!/usr/bin/env python3
"""Synthetic queue benchmarks using normal deployments, CLI workers and PostgreSQL.

Python >=3.11 and psql are required; run inside the project's Nix environment.
No build is performed. See benchmarks/queue/README.md for methodology.
"""
import argparse
import csv
from datetime import datetime
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import platform
import signal
import shutil
import socket
import statistics
import subprocess
import sys
import time
import tomllib
import uuid
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MAX_SECONDS = 300


def timing(per_sample=0.0, overhead=0.0, noise=0.0, seed=0):
    return dict(per_sample_seconds=per_sample, overhead_seconds=overhead,
                sigma_per_sample_seconds=per_sample * noise,
                sigma_overhead_seconds=overhead * noise, seed=seed)


def cases(suite):
    return [dict(evaluators=e, rate=r, regime=g, continuous_dims=suite['continuous_dims'],
                 noise_fraction=suite['noise_fraction'])
            for e, r, g in itertools.product(suite['evaluators'], suite['rates'], suite['regimes'])]


def case_id(case):
    return f"{case['regime']}-e{case['evaluators']}-r{case['rate']}"


def inline(value):
    if isinstance(value, dict):
        return '{ ' + ', '.join(f'{k} = {inline(v)}' for k, v in value.items()) + ' }'
    return json.dumps(value, allow_nan=False)


def run_card(case, seed, name, budget):
    e, rate, regime = case['evaluators'], case['rate'], case['regime']
    noise = case['noise_fraction']
    sampler = dict(kind='naive_monte_carlo', seed=seed,
                   generation_timing=timing(0 if regime in ('inference','training_burst') else (2 / rate if regime == 'sampler_limited' else 1 / (8 * rate)), noise=noise, seed=seed + 1))
    if regime.startswith('training_'):
        window = max(64, math.ceil(rate * (8 if regime == 'training_large' else .25)))
        if regime == 'training_burst': window = min(window,100000)
        sampler.update(training_window_samples=window,
                       ingest_timing=timing(0, 0, 0, seed + 2) if regime == 'training_burst' else timing(.01 / rate, .0001, noise, seed + 2),
                       update_timing=timing(0, .01 if regime == 'training_large' else .5, noise, seed + 3))
    return f'''name = {inline(name)}
[evaluator]
kind = "unit"
continuous_dims = {case['continuous_dims']}
timing = {inline(timing(e / rate, .25 if regime == 'batch_overhead' else .001, noise, seed))}

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
stop_condition = {{ max_samples = {budget} }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {inline(sampler)} }}
'''


def command(args, **kwargs):
    kwargs.setdefault("timeout", 120)
    return subprocess.check_output([str(x) for x in args], text=True, **kwargs)


def parse_cli_json(text):
    # Commands such as `node stop name1 name2` emit one JSON value per node.
    decoder = json.JSONDecoder()
    values = []
    while text.strip():
        value, end = decoder.raw_decode(text.lstrip())
        values.append(value)
        text = text.lstrip()[end:]
    return values[0] if len(values) == 1 else values


def sql_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def wait_for(probe, timeout=90):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        result = probe()
        if result:
            return result
        time.sleep(.2)
    raise TimeoutError('benchmark readiness/shutdown deadline exceeded')


def port_in_use(port):
    with socket.socket() as sock:
        return sock.connect_ex(('127.0.0.1', port)) == 0


class Deployment:
    def __init__(self, binary, offset, directory):
        self.binary = Path(binary).resolve()
        self.directory = directory
        self.offset = offset
        self.url = f'postgresql://postgres@127.0.0.1:{5400 + offset}/gammaboard_db'
        self.process = None
        self.workers = []
        self.run_name = None
        self.log = None
        self.runtime = directory / 'runtime.toml'

    def cli(self, *args):
        return parse_cli_json(command([self.binary, '--runtime-config', self.runtime,
                                   '--port-offset', self.offset, '--json', *args], cwd=ROOT))

    def sql(self, query):
        return command(['psql', self.url, '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1'],
                       input=query, stderr=self.log).strip()

    def __enter__(self):
        for port in (8080 + self.offset, 4000 + self.offset, 5400 + self.offset):
            if port_in_use(port):
                raise RuntimeError(f'port {port} already in use; refusing to touch an existing deployment')
        self.directory.mkdir(parents=True, exist_ok=True)
        # Separate resource root and PostgreSQL capacity for 65 ordinary workers.
        resources = self.directory / 'resources'
        self.runtime.write_text(f'''[resources]
roots = [{inline(str(resources))}]
[local_postgres]
max_connections = 512
''')
        self.log = (self.directory / 'deploy.log').open('w')
        self.process = subprocess.Popen([str(self.binary), '--runtime-config', str(self.runtime),
            '--port-offset', str(self.offset), 'deploy'], cwd=ROOT, stdout=self.log, stderr=subprocess.STDOUT)
        try:
            def ready():
                if self.process.poll() is not None:
                    raise RuntimeError(f'deployment exited; see {self.directory / "deploy.log"}')
                try:
                    # PostgreSQL accepts connections before deploy finishes migrations.
                    # The API is only started after schema initialization completes.
                    with urllib.request.urlopen(f'http://127.0.0.1:{4000+self.offset}/api/health',timeout=1) as response:
                        return json.load(response).get('database') == 'connected'
                except (OSError,ValueError):
                    return False
            wait_for(ready)
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def wait_workers(self, predicate):
        names = ','.join(sql_literal(n) for n in self.workers)
        return self.sql(f"select count(*) from nodes where name in ({names}) and {predicate};")

    def stop_workers(self):
        if self.workers:
            self.cli('node', 'stop', *self.workers)
            wait_for(lambda: self.wait_workers('lease_expires_at>now()') == '0')
            self.workers = []

    def ensure_workers(self, evaluators):
        if len(self.workers) != evaluators + 1:
            self.stop_workers()
            self.workers = self.cli('node', 'start-local', str(evaluators + 1))['node_names']
        wait_for(lambda: self.wait_workers('lease_expires_at>now()') == str(evaluators + 1))

    def clear_case(self):
        if self.run_name:
            self.cli('run', 'pause', self.run_name)
            if self.workers:
                wait_for(lambda: self.wait_workers('active_run_id is not null and lease_expires_at>now()') == '0')
            self.cli('run', 'remove', '--yes', self.run_name)
            self.run_name = None

    def __exit__(self, *exc):
        try:
            try:
                self.clear_case()
            finally:
                self.stop_workers()
        finally:
            if self.process and self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    raise RuntimeError(f'graceful shutdown timed out; inspect PID {self.process.pid}')
            if self.log:
                self.log.close()

    def observe(self, run_id):
        query = f'''select json_build_object(
            'progress',(select nr_completed_samples from run_tasks where run_id={run_id} and task->>'kind'='sample' limit 1),
            'state',(select state from run_tasks where run_id={run_id} and task->>'kind'='sample' limit 1),
            'active_evaluators',(select count(*) from nodes where active_run_id={run_id} and active_role='evaluator' and lease_expires_at>now()),
            'queue',(select json_object_agg(status,n) from (select status,count(*) n from batches where run_id={run_id} group by status) q),
            'sampler',(select row_to_json(s) from sampler_aggregator_performance_latest s where run_id={run_id} order by created_at desc limit 1),
            'evaluators',(select json_agg(row_to_json(e)) from evaluator_performance_latest e where run_id={run_id}));'''
        start = time.monotonic()
        row = json.loads(self.sql(query))
        row['monotonic'] = (start + time.monotonic()) / 2
        return row

    def measure(self, case, seed, warmup, duration, output):
        name = 'synthetic-' + case_id(case) + '-' + uuid.uuid4().hex[:8]
        budget = math.ceil(case['rate'] * (warmup + duration + 60) * 2)
        card = run_card(case, seed, name, budget)
        card_path = output.with_suffix('.toml')
        card_path.write_text(card)
        created = self.cli('run', 'create', card_path)
        self.run_name = name
        run_id = created['run_id']
        try:
            self.ensure_workers(case['evaluators'])
            self.cli('run', 'resume', str(run_id), '--max-evaluators', str(case['evaluators']))
            wait_for(lambda: self.sql(f"select count(*) from nodes where active_run_id={run_id} and lease_expires_at>now();") == str(case['evaluators'] + 1))
            rows = []
            start = time.monotonic()
            with output.with_suffix('.jsonl').open('w') as raw:
                while True:
                    row = self.observe(run_id)
                    row['elapsed'] = row['monotonic'] - start
                    raw.write(json.dumps(row, allow_nan=False) + '\n')
                    raw.flush()
                    if row['state'] != 'active':
                        raise RuntimeError(f'benchmark task became {row["state"]}; inspect {raw.name}')
                    if row['active_evaluators'] != case['evaluators']:
                        raise RuntimeError('benchmark evaluator count changed during measurement')
                    rows.append(row)
                    if row['elapsed'] >= warmup + duration:
                        break
                    time.sleep(.1)
            with output.with_suffix('.csv').open('w',newline='') as trace:
                writer=csv.writer(trace)
                writer.writerow(['elapsed_seconds','completed_samples_per_second','produced_samples_per_second','training_updates','pending_batches','claimed_batches'])
                for previous,current in zip(rows,rows[1:]):
                    dt=current['monotonic']-previous['monotonic']
                    before=((previous['sampler'] or {}).get('engine_diagnostics') or {})
                    after=((current['sampler'] or {}).get('engine_diagnostics') or {})
                    queue=current['queue'] or {}
                    writer.writerow([current['elapsed'],(current['progress']-previous['progress'])/dt,
                                     (after.get('produced_samples',0)-before.get('produced_samples',0))/dt,
                                     after.get('training_updates',0),queue.get('pending',0),queue.get('claimed',0)])
            measured = [r for r in rows if r['elapsed'] >= warmup]
            seconds = measured[-1]['monotonic'] - measured[0]['monotonic']
            rate = (measured[-1]['progress'] - measured[0]['progress']) / seconds
            if rate <= 0:
                raise RuntimeError('no completed samples in the measurement interval')
            busy = [r['sampler']['runtime_metrics'].get('avg_evaluator_utilization') for r in measured if r['sampler']]
            busy = [v for v in busy if v is not None]
            # Cumulative evaluator counters provide actual mean batch size.
            def counters(row):
                metrics = [e['metrics'] for e in row['evaluators'] or []]
                return sum(e['samples_evaluated'] for e in metrics), sum(e['batches_completed'] for e in metrics)
            n0, b0 = counters(measured[0]); n1, b1 = counters(measured[-1])
            def barrier(row):
                return ((row['sampler'] or {}).get('engine_diagnostics') or {}).get('training_barrier_seconds',0)
            first_sampler, last_sampler = measured[0]['sampler'], measured[-1]['sampler']
            barrier_span = ((datetime.fromisoformat(last_sampler['created_at']) - datetime.fromisoformat(first_sampler['created_at'])).total_seconds()
                            if first_sampler and last_sampler else None)
            diagnostics = lambda r: ((r['sampler'] or {}).get('engine_diagnostics') or {})
            first_diag, last_diag = diagnostics(measured[0]), diagnostics(measured[-1])
            updates = last_diag.get('training_updates',0) - first_diag.get('training_updates',0)
            update_seconds = (last_diag.get('update_timing') or {}).get('actual_seconds',0) - (first_diag.get('update_timing') or {}).get('actual_seconds',0)
            if case['regime'] == 'training_burst' and updates < 2:
                raise RuntimeError('burst benchmark did not observe two complete update stalls; increase its measurement duration')
            return dict(case=case, seed=seed, seconds=seconds, samples_per_second=rate,
                        training_updates=updates, mean_update_seconds=update_seconds/updates if updates else None,
                        recorded_training_barrier_seconds=max(0,barrier(measured[-1])-barrier(measured[0])) if barrier_span is not None else None,
                        barrier_observation_seconds=barrier_span,
                        compute_ceiling_fraction=rate / case['rate'],
                        samples_per_evaluator_second=rate / case['evaluators'],
                        mean_busy=statistics.mean(busy) if busy else None,
                        mean_batch_size=(n1-n0)/(b1-b0) if b1>b0 else None,
                        queue_peak=max(sum((r['queue'] or {}).values()) for r in measured),
                        final_sampler=measured[-1]['sampler'],
                        final_evaluators=measured[-1]['evaluators'])
        finally:
            self.clear_case()


def identity(binary):
    path = Path(binary).resolve()
    with path.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    return dict(binary=str(path), sha256=digest)


def snapshot_binaries(variants, output):
    directory = output / 'binaries'
    directory.mkdir()
    snapshots, metadata = [], {}
    for label, source in variants:
        destination = directory / label
        shutil.copy2(Path(source).resolve(), destination)
        snapshots.append((label,destination))
        metadata[label] = dict(identity(destination), source_binary=str(Path(source).resolve()))
    return snapshots, metadata


def load_records(output):
    path = output / 'results.jsonl'
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def append_record(output, record):
    # Only finished, cleaned-up cases are durable checkpoints. Raw observations
    # from an interrupted case are retained in its separate attempt directory.
    with (output / 'results.jsonl').open('a') as stream:
        stream.write(json.dumps(record, allow_nan=False) + '\n')
        stream.flush()
        os.fsync(stream.fileno())


def summary(records):
    groups = {}
    for r in records:
        groups.setdefault((r['variant'], case_id(r['case'])), []).append(r['samples_per_second'])
    return [dict(variant=v, case=c, repetitions=len(xs), mean=statistics.mean(xs),
                 stdev=statistics.stdev(xs) if len(xs)>1 else None,
                 minimum=min(xs), maximum=max(xs)) for (v,c),xs in groups.items()]


def write_summary(output, records):
    rows = summary(records)
    (output/'summary.json').write_text(json.dumps(rows,indent=2))
    lines = ['# Synthetic queue benchmark', '', '| Variant | Case | Repetitions | Mean samples/s | Standard deviation | Updates | Mean update stall (s) |', '| --- | --- | ---: | ---: | ---: | ---: | ---: |']
    for row in rows:
        spread = f"{row['stdev']:.1f}" if row['stdev'] is not None else '—'
        matching = [r for r in records if r['variant']==row['variant'] and case_id(r['case'])==row['case']]
        updates = sum(r.get('training_updates',0) for r in matching)
        stall = sum(r['mean_update_seconds']*r['training_updates'] for r in matching if r.get('mean_update_seconds') is not None)/updates if updates else 0
        lines.append(f"| {row['variant']} | {row['case']} | {row['repetitions']} | {row['mean']:.1f} | {spread} | {updates} | {stall:.3f} |")
    lines += ['', 'Short runs are coarse regression measurements, not precision estimates. Repeat paired runs before interpreting small changes.', 'Compute ceilings exclude sampler costs, batch overhead and training barriers. Raw observations and binary hashes are retained alongside this report.', '']
    (output/'summary.md').write_text('\n'.join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['generate','run','ab'])
    parser.add_argument('--suite', type=Path, default=ROOT/'benchmarks/queue/suite.toml')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--smoke', action='store_true')
    parser.add_argument('--resume', action='store_true', help='continue an existing run with the same settings and saved binaries')
    parser.add_argument('--evaluators', type=int, nargs='+')
    parser.add_argument('--rates', type=int, nargs='+')
    parser.add_argument('--regimes', nargs='+')
    parser.add_argument('--noise', type=float)
    parser.add_argument('--warmup', type=float)
    parser.add_argument('--duration', type=float)
    parser.add_argument('--repetitions', type=int)
    parser.add_argument('--port-offset', type=int, default=30)
    args = parser.parse_args()
    started = time.monotonic()
    suite = tomllib.loads(args.suite.read_text())
    for key in ('evaluators','rates','regimes'):
        if getattr(args,key) is not None: suite[key] = getattr(args,key)
    if args.noise is not None: suite['noise_fraction'] = args.noise
    matrix = cases(suite)
    if args.smoke:
        wanted = {('inference',1,1000),('training_burst',4,2000000),('inference',16,2000000),('training_burst',64,1000)}
        matrix = [c for c in matrix if (c['regime'],c['evaluators'],c['rate']) in wanted]
    if not matrix: parser.error('no matching benchmark cases')
    if len({case_id(c) for c in matrix}) != len(matrix): parser.error('duplicate benchmark cases')
    if suite['continuous_dims'] < 1: parser.error('benchmark requires at least one continuous dimension')
    if any(c['evaluators']<1 or c['rate']<1 or c['regime'] not in ['inference','batch_overhead','sampler_limited','training_large','training_small','training_burst'] for c in matrix): parser.error('invalid workload parameters')
    if not math.isfinite(suite['noise_fraction']) or suite['noise_fraction']<0: parser.error('noise must be finite and nonnegative')
    warmup = args.warmup if args.warmup is not None else suite['warmup_seconds']
    duration = args.duration if args.duration is not None else suite['measurement_seconds']
    repeats = args.repetitions if args.repetitions is not None else (1 if args.smoke else suite['repetitions'])
    if not all(math.isfinite(x) for x in [warmup,duration]) or warmup<0 or duration<1 or repeats<1 or not 1<=args.port_offset<=57000: parser.error('invalid duration, repetitions or port offset')
    requested_variants = [] if args.action == 'generate' else ([('run',args.binary)] if args.action == 'run' else [('A',args.baseline),('B',args.candidate)])
    if any(p is None or not p.is_file() or not os.access(p,os.X_OK) for _,p in requested_variants): parser.error('supply executable --binary, or --baseline and --candidate files')
    measurement_budget = len(matrix)*repeats*len(requested_variants)*(warmup+duration)
    if measurement_budget+30 > MAX_SECONDS: parser.error('requested measurements exceed the wall-time budget; reduce cases, duration or repetitions')
    args.output = args.output.resolve()
    if args.resume and args.action == 'generate': parser.error('generate cannot be resumed')
    args.output.mkdir(parents=True, exist_ok=args.resume)
    manifest = dict(suite=suite, cases=matrix, warmup=warmup, duration=duration, repetitions=repeats,
                    host=platform.node(), platform=platform.platform(), cpu_count=os.cpu_count(), port_offset=args.port_offset,
                    cpu_affinity=sorted(os.sched_getaffinity(0)) if hasattr(os,'sched_getaffinity') else None,
                    thread_limits={k:os.environ.get(k) for k in ['OMP_NUM_THREADS','RAYON_NUM_THREADS']},
                    source_commit=command(['git','rev-parse','HEAD'],cwd=ROOT).strip(),
                    source_dirty=bool(command(['git','status','--porcelain'],cwd=ROOT).strip()),
                    started_unix=time.time(), max_seconds=MAX_SECONDS,
                    runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                    suite_sha256=hashlib.sha256(args.suite.read_bytes()).hexdigest(), description='Synthetic compute ceilings exclude batch overhead, sampler costs and barriers.')
    if args.action == 'generate':
        (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2))
        for case in matrix:
            (args.output/(case_id(case)+'.toml')).write_text(run_card(case,1234,'synthetic-'+case_id(case),math.ceil(case['rate']*(warmup+duration+60)*2)))
        print(f'Generated {len(matrix)} run cards in {args.output}')
        return
    if args.resume:
        saved = json.loads((args.output/'manifest.json').read_text())
        for key in ('cases','warmup','duration','repetitions','host','cpu_affinity','thread_limits','port_offset','runner_sha256','suite_sha256'):
            if saved[key] != manifest[key]: parser.error(f'resume mismatch: {key}')
        variants = []
        if set(saved['binaries']) != {label for label, _ in requested_variants}:
            parser.error('resume mismatch: variants')
        for label, source in requested_variants:
            binary = saved['binaries'][label]
            if identity(source)['sha256'] != binary['sha256'] or identity(binary['binary'])['sha256'] != binary['sha256']:
                parser.error(f'resume mismatch: binary {label}')
            variants.append((label,Path(binary['binary'])))
    else:
        variants, manifest['binaries'] = snapshot_binaries(requested_variants,args.output)
        (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2))
    def deadline(*_):
        raise TimeoutError('benchmark wall-time budget exhausted; partial results retained, cleaning up')
    signal.signal(signal.SIGALRM,deadline)
    signal.setitimer(signal.ITIMER_REAL,max(.1,MAX_SECONDS-(time.monotonic()-started)-30))
    records = load_records(args.output)
    completed = {(r['variant'],r['repetition'],case_id(r['case'])) for r in records}
    for repeat in range(repeats):
        # Alternate A/B deployment order; each pair receives identical workload seeds.
        order = variants if repeat%2==0 else list(reversed(variants))
        for label,binary in order:
            remaining = [case for case in matrix if (label,repeat,case_id(case)) not in completed]
            if not remaining: continue
            directory = args.output/f'{label}-{repeat}-{uuid.uuid4().hex[:8]}'
            with Deployment(binary,args.port_offset,directory) as deployment:
                for case in remaining:
                    if time.monotonic()-started+warmup+duration+30 > MAX_SECONDS:
                        raise TimeoutError('insufficient time for another case and cleanup')
                    cid=case_id(case)
                    seed=1234+repeat
                    print(f'{label} repetition {repeat+1}: {cid}',flush=True)
                    result=deployment.measure(case,seed,warmup,duration,directory/cid)
                    result.update(variant=label,repetition=repeat)
                    append_record(args.output,result)
                    records.append(result)
                    write_summary(args.output,records)
                    print(f'  {result["samples_per_second"]:.1f} samples/s',flush=True)
    write_summary(args.output,records)
    if args.action=='ab':
        pairs=[]
        for case in matrix:
            a={r['repetition']:r['samples_per_second'] for r in records if r['variant']=='A' and r['case']==case}
            b={r['repetition']:r['samples_per_second'] for r in records if r['variant']=='B' and r['case']==case}
            ratios=[b[k]/a[k] for k in sorted(a.keys()&b.keys())]
            pairs.append(dict(case=case_id(case), paired_ratios=ratios, mean_ratio=statistics.mean(ratios), stdev_ratio=statistics.stdev(ratios) if len(ratios)>1 else None))
        (args.output/'comparison.json').write_text(json.dumps(pairs,indent=2))
        with (args.output/'summary.md').open('a') as report:
            report.write('\n## Paired comparison\n\n| Case | Mean B/A | Ratio standard deviation |\n| --- | ---: | ---: |\n')
            for pair in pairs:
                spread = f"{pair['stdev_ratio']:.4f}" if pair['stdev_ratio'] is not None else '—'
                report.write(f"| {pair['case']} | {pair['mean_ratio']:.4f} | {spread} |\n")
            report.write('\nRatios above 1 favor B; assess variation and repeatability before concluding an improvement.\n')

    signal.setitimer(signal.ITIMER_REAL,0)
    elapsed=time.monotonic()-started
    (args.output/'completion.json').write_text(json.dumps(dict(elapsed_seconds=elapsed,measurements=len(records),expected_measurements=len(matrix)*repeats*len(variants),within_budget=elapsed<=MAX_SECONDS),indent=2))
    if elapsed>MAX_SECONDS: raise RuntimeError('benchmark exceeded its wall-time budget')
    print(f'Completed {len(records)} measurements in {elapsed:.1f}s (including deployment and cleanup).',flush=True)


def interrupted(*_):
    raise KeyboardInterrupt()


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, interrupted)
    main()
