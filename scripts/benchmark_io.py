"""Focused insert-concurrency experiments, using the shared benchmark lifecycle."""
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import time


def activity(measurement):
    first, last = measurement['snapshots'][0], measurement['snapshots'][-1]
    result = {}
    for collection, field, role in [('evaluators','metrics','evaluator'),('samplers','runtime_metrics','sampler')]:
        start = {r['worker_id']: r[field] for r in first[collection]}
        total = dict(elapsed_seconds=0., compute_seconds=0., io_seconds=0.)
        for row in last[collection]:
            a, b = start[row['worker_id']], row[field]
            epoch = 'epoch' if role == 'evaluator' else 'runner_epoch'
            if any(a.get(k) != b.get(k) for k in [epoch,'node_uuid','task_id']):
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


def summarize(records):
    groups = {}
    for row in records:
        groups.setdefault((row['workers'],row['batch_size'],row['inserts']),[]).append(row)
    result = []
    for (workers,batch,inserts), rows in sorted(groups.items()):
        valid = [r for r in rows if r['valid']]
        stats = {}
        for field in ['rate','batches_per_second','input_payload_bytes_per_batch',
                      'accepted_input_mib_per_second','evaluator_compute','evaluator_io',
                      'sampler_compute','sampler_io']:
            values = [r[field] for r in valid if r.get(field) is not None]
            stats[field] = dict(median=statistics.median(values),minimum=min(values),maximum=max(values)) if values else None
        result.append(dict(workers=workers,batch_size=batch,inserts=inserts,
                           valid_trials=len(valid),invalid_trials=len(rows)-len(valid),**stats))
    return result


def execute(args):
    import benchmark as bench
    if not (1 <= args.port_offset <= 57000 and 1 <= args.cpu_limit <= 8 and 1 <= args.repetitions <= 5
            and math.isfinite(args.duration) and math.isfinite(args.warmup)
            and args.duration >= 4 and args.warmup >= 1 and args.iterations >= 0 and args.min_tick_ms >= 0):
        raise ValueError('invalid experiment settings')
    if any(n<1 or n>64 for n in args.workers) or any(n<16 or n>1000000 for n in args.batch_sizes):
        raise ValueError('use 1–64 evaluators and batches of at least 16')
    if len(set(args.workers)) != len(args.workers) or len(set(args.batch_sizes)) != len(args.batch_sizes):
        raise ValueError('duplicate matrix entries')
    if not args.inserts or len(set(args.inserts)) != len(args.inserts) or any(n<1 or n>8 for n in args.inserts):
        raise ValueError('use unique insert limits between 1 and 8')
    if not 1 <= args.insert_bundle_size <= 100:
        raise ValueError('insert bundle size must be between 1 and 100')
    cases = list(itertools.product(args.workers,args.batch_sizes,args.inserts,range(args.repetitions)))
    budget = len(cases)*(args.duration+args.warmup+5)+90
    if budget>1800: raise ValueError('matrix exceeds the 30-minute budget')
    candidates = bench.physical_cpus()
    cpus = bench.idle_cpus(candidates,min(args.cpu_limit,len(candidates)))
    os.sched_setaffinity(0,cpus); os.nice(5)
    for k in ['OMP_NUM_THREADS','RAYON_NUM_THREADS','OPENBLAS_NUM_THREADS','TOKIO_WORKER_THREADS']:
        os.environ[k]='1'
    binary=args.binary.resolve(strict=True)
    args.output.mkdir(parents=True,exist_ok=False); out=args.output.resolve()
    with binary.open('rb') as f: digest=hashlib.file_digest(f,'sha256').hexdigest()
    migrations=Path(os.environ.get('GAMMABOARD_MIGRATIONS_DIR',bench.ROOT/'migrations'))
    manifest=dict(cpus=cpus,binary=str(binary),binary_sha256=digest,workers=args.workers,
        batch_sizes=args.batch_sizes,repetitions=args.repetitions,duration=args.duration,
        warmup=args.warmup,cpu_iterations_per_sample=args.iterations,min_tick_ms=args.min_tick_ms,planned_cases=len(cases),
        insert_limits=args.inserts,insert_bundle_size=args.insert_bundle_size,input_storage=args.input_storage,
        migrations_directory=os.environ.get('GAMMABOARD_MIGRATIONS_DIR'),
        migration_files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(migrations.glob('*.sql'))},
        scope='Insert scheduling comparison; fleets share a fixed CPU budget, not strong scaling.',
        payload_scope='Full six-dimensional indexed inputs, scalar inference results. Accepted input MiB/s excludes protocol, WAL, retries and result traffic.',
        started_at=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),status='running')
    bench.write_json(out/'manifest.json',manifest)
    rng=random.Random(1234); ordered=[]; records=[]
    for repeat in range(args.repetitions):
        block=[c for c in cases if c[-1]==repeat]; rng.shuffle(block); ordered.extend(block)
    try:
        with bench.Session(binary,out,budget,args.port_offset,
                max_connections=max(128,4*(max(args.workers)+1)+32)) as session:
            if args.input_storage != 'default':
                sql = ('ALTER TABLE batch_inputs ALTER COLUMN latent_batch SET STORAGE EXTERNAL;'
                       if args.input_storage == 'external' else
                       'ALTER TABLE batch_inputs ALTER COLUMN latent_batch SET STORAGE EXTENDED; '
                       f'ALTER TABLE batch_inputs ALTER COLUMN latent_batch SET COMPRESSION {args.input_storage};')
                changed = subprocess.run(['psql','-X','-v','ON_ERROR_STOP=1',
                    f'postgresql://postgres@127.0.0.1:{5400+args.port_offset}/gammaboard_db','-c',sql],
                    capture_output=True,text=True,timeout=10,check=True)
                (out/'input-storage.log').write_text(changed.stdout+changed.stderr)
            session.cli('node','start-local',max(args.workers)+1)
            for index,(workers,batch,inserts,repeat) in enumerate(ordered):
                directory=out/f'case-{index:03d}'; directory.mkdir()
                card=directory/'run.toml'
                text=bench.run_card(args.iterations,batch,min_tick_time_ms=args.min_tick_ms,telemetry_interval_ms=500)
                card.write_text(text.replace('[sampler_aggregator_runner_params.queue]',
                    f'[sampler_aggregator_runner_params.queue]\nmax_concurrent_insert_tasks = {inserts}\n'
                    f'max_insert_bundle_size = {args.insert_bundle_size}'))
                run=session.cli('run','create',card)['run_id']
                session.cli('run','resume',run,'--max-evaluators',workers)
                session.cli('run','wait',run,'--until','ready','--evaluators',workers,'--max-age','5s')
                for name,seconds in [('warmup',args.warmup),('measurement',args.duration)]:
                    measured=session.cli('run','performance',run,'--duration',f'{seconds}s',
                                         '--interval','500ms','--max-age','5s')
                    bench.write_json(directory/f'{name}.json',measured)
                record=dict(case=index,workers=workers,batch_size=batch,inserts=inserts,repeat=repeat,
                    valid=measured['valid'],issues=measured['issues'],rate=measured['samples_per_second'])
                try:
                    record.update(activity(measured))
                    if record['rate'] is not None:
                        record.update(payload_throughput(measured,batch))
                except (ValueError,KeyError,ZeroDivisionError,TypeError) as exc:
                    record.update(valid=False,issues=[*record['issues'],str(exc)])
                if record['rate'] is None or not math.isfinite(record['rate']) or record['rate']<=0:
                    record['valid']=False
                    record['issues'].append('no finite positive throughput')
                records.append(record)
                with (out/'results.jsonl').open('a') as f: f.write(json.dumps(record,allow_nan=False)+'\n')
                bench.write_json(out/'summary.json',summarize(records))
                print(f'{index+1}/{len(cases)}: workers={workers} batch={batch} inserts={inserts} '+
                    (f'{record["rate"]:,.0f} samples/s; {record["batches_per_second"]:,.1f} batches/s; '
                     f'{record["accepted_input_mib_per_second"]:,.1f} input MiB/s'
                     if record['valid'] else f'INVALID {record["issues"]}'),flush=True)
                session.cli('run','pause',run); session.cli('run','wait',run,'--until','idle')
                session.cli('run','remove','--yes',run)
        manifest['status']='completed' if all(r['valid'] for r in records) else 'completed_with_invalid_cases'
    except BaseException as exc:
        manifest.update(status='incomplete',error=str(exc) or type(exc).__name__); raise
    finally:
        manifest.update(completed_cases=len(records),finished_at=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
        bench.write_json(out/'manifest.json',manifest); bench.write_json(out/'summary.json',summarize(records))
    return out
