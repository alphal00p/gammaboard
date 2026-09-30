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
import statistics
import subprocess
import sys
import time
import tomllib

from benchmark_common import (ROOT, Session, active_run, calibrate_workloads, idle_cpus, load_results, physical_cpus,
                              preserve_inputs, run_card, workload, write_json)


def validate_suite(s):
    allowed = {'eval_us', 'workers', 'batch_sizes', 'repetitions', 'cpu_limit',
               'duration_seconds', 'warmup_seconds', 'budget_seconds', 'seed',
               'min_tick_time_ms', 'telemetry_interval_ms',
               'generation_batch_size'}
    if unknown := s.keys() - allowed:
        raise ValueError(f'unknown suite settings: {sorted(unknown)}')
    for key in ('eval_us', 'workers', 'batch_sizes'):
        values = s.get(key)
        types = (int, float) if key == 'eval_us' else (int,)
        if not isinstance(values, list) or not values or any(type(v) not in types or not math.isfinite(v) or v <= 0 for v in values) or len(set(values)) != len(values):
            raise ValueError(f'{key} must contain unique positive finite values')
    if min(s['eval_us']) < .1 or max(s['eval_us']) > 100000:
        raise ValueError('eval_us must be between 0.1 and 100000')
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
    generation_size = s.get('generation_batch_size', max(s['batch_sizes']))
    if type(generation_size) is not int or not max(s['batch_sizes']) <= generation_size <= 1000000:
        raise ValueError('generation_batch_size must cover all evaluator batches and be at most 1000000')
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


def measure_case(session, directory, suite, cost, workers, batch_size, repeat, iterations):
    evaluator_card = directory/'evaluator.toml'
    evaluator_card.write_text(workload(iterations))
    card = directory/'run.toml'
    card.write_text(run_card(iterations, batch_size, suite.get('min_tick_time_ms', 10),
                             suite.get('telemetry_interval_ms', 250),
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
        with active_run(session, card, workers, max_age) as run_id:
            # Readiness and discarded warmup are distinct. Save both the warmup and
            # measurement so startup effects can be inspected without repeating work.
            for name, duration in [('warmup', suite['warmup_seconds']),
                                   ('gammaboard', suite['duration_seconds'])]:
                observation = session.cli('run', 'performance', run_id, '--duration', f'{duration}s',
                                           '--interval', interval, '--max-age', max_age)
                write_json(directory/f'{name}.json', observation)
                if name == 'gammaboard':
                    measurement = observation
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
    binary, harness_hashes = preserve_inputs(output, binary)
    with binary.open('rb') as binary_file:
        binary_hash = hashlib.file_digest(binary_file,'sha256').hexdigest()
    metadata = dict(schema_version=1, experiment='matrix', suite=suite, cpus=cpus, physical_core_budget=len(cpus),
                    cpu_model=next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), None),
                    load_average=os.getloadavg(), binary=str(binary), binary_sha256=binary_hash,
                    harness_files=harness_hashes,
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
            calibrations = calibrate_workloads(session, suite['eval_us'], args.calibration)
            write_json(output/'calibration.json', calibrations)
            session.cli('node','start-local',max(suite['workers'])+1)
            for index, (cost, workers, batch_size, repeat) in enumerate(cases):
                directory = output/f'case-{index:03d}'
                directory.mkdir()
                iterations = calibrations[str(float(cost))]['cpu_iterations_per_sample']
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
    frontier = commands.add_parser('frontier', help='sparse throughput curves for RNG, materialized and feedback data paths')
    frontier.add_argument('suite', type=Path, nargs='?', default=ROOT/'resources/templates/benchmarks/frontier.toml')
    frontier.add_argument('--binary', type=Path, default=ROOT/'target/dev-optim/gammaboard')
    frontier.add_argument('--output', type=Path, required=True)
    frontier.add_argument('--port-offset', type=int, default=150)
    frontier.add_argument('--budget', type=int, help='override the suite time limit; incomplete coverage remains visible')
    design = frontier.add_mutually_exclusive_group()
    design.add_argument('--points', type=Path, help='JSON list of explicit points to validate in fresh runs')
    design.add_argument('--search', action='store_true', help='opt in to adaptive evaluator-batch and worker-count exploration')
    process = commands.add_parser('process', help='targeted Rust/Python process API correctness and overhead tests')
    process.add_argument('--output', type=Path, required=True)
    process.add_argument('--python', type=Path, help='Python interpreter with NumPy')
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
            if 'modes' in suite:
                import benchmark_frontier
                benchmark_frontier.validate(suite)
                points = benchmark_frontier.sparse_points(suite)
                measured = sum(benchmark_frontier.measurement_seconds(suite,p,'confirm') for p in points)
                print(f'{len(points)} sparse fresh-run measurements; {measured/60:.1f} min of measurement windows '
                      f'plus startup/warmup/drain; {suite["budget_seconds"]/60:.1f} min time limit; '
                      f'up to {max(suite["workers"])} evaluators. Adaptive tuning requires --search.')
            else:
                cases = validate_suite(suite)
                print(f'{len(cases)} trials; estimated {estimated_seconds(suite)/60:.1f} min; '
                      f'hard budget {suite["budget_seconds"]/60:.1f} min; {suite["cpu_limit"]} physical cores')
        elif args.command=='run':
            if not 1 <= args.port_offset <= 57000: raise ValueError('invalid port offset')
            print(execute(args))
        elif args.command=='io':
            import benchmark_io
            print(benchmark_io.execute(args))
        elif args.command=='frontier':
            import benchmark_frontier
            if not 1 <= args.port_offset <= 57000: raise ValueError('invalid port offset')
            print(benchmark_frontier.execute(args))
        elif args.command=='process':
            import benchmark_process
            print(benchmark_process.execute(args))
        elif args.command in ('plot', 'summary'):
            kind = artifact_kind(args.directory)
            if kind == 'process_api':
                import benchmark_process
                print(benchmark_process.report(args.directory, plots=args.command == 'plot'))
            elif kind == 'frontier':
                import benchmark_frontier_plots
                print(benchmark_frontier_plots.report(args.directory, plots=args.command == 'plot'))
            elif kind == 'io':
                import benchmark_io
                print(benchmark_io.report(args.directory, plots=args.command == 'plot'))
            elif args.command == 'plot':
                print(plot(args.directory))
            else:
                summary(args.directory)
        else:
            kind = artifact_kind(args.before)
            if kind != artifact_kind(args.after):
                raise ValueError('cannot compare different benchmark families')
            if kind == 'frontier':
                import benchmark_frontier
                benchmark_frontier.compare(args.before, args.after)
            elif kind == 'io':
                import benchmark_io
                benchmark_io.compare(args.before, args.after)
            else:
                compare(args.before,args.after)
    except (ValueError, RuntimeError, TimeoutError, OSError, subprocess.SubprocessError, KeyboardInterrupt) as exc:
        print(f'Benchmark stopped: {str(exc) or type(exc).__name__}',file=sys.stderr)
        return 1
    return 0


def artifact_kind(directory):
    manifest = json.loads((directory/'manifest.json').read_text())
    return manifest.get('experiment') or ('frontier' if manifest.get('schema_version') == 2
                                         else 'io' if 'insert_limits' in manifest else 'matrix')


def interrupted(*_):
    raise KeyboardInterrupt()


if __name__=='__main__':
    signal.signal(signal.SIGTERM,interrupted)
    sys.exit(main())
