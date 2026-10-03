"""Controlled generation-size comparison; run from the GammaBoard repository."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import time
import tomllib

sys.path.insert(0, str(Path.cwd()))
from benchmarks import amortization, common as bench, frontier

GENERATIONS = [131072, 262144, 2097152]
WORKERS = 16
BATCH = 32768
ITERATIONS = 3051


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary, hashes = bench.preserve_inputs(output, args.binary)
    shutil.copy2(__file__, output / 'inputs/run_study.py')
    candidates = bench.physical_cpus()
    if len(candidates) < WORKERS + 9:
        raise ValueError('This study needs 25 physical cores')
    loads = bench.core_loads(candidates, seconds=2)
    cpus = sorted(sorted(candidates, key=lambda c: (loads[c], c))[:WORKERS + 9])
    sampler, database, evaluators = cpus[:5], cpus[5:9], cpus[9:]
    for key in ['OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'MKL_NUM_THREADS', 'TOKIO_WORKER_THREADS']:
        os.environ[key] = '1'
    suite = tomllib.loads((bench.ROOT / 'benchmarks/frontier.toml').read_text())
    suite.update(workers=[WORKERS], sampler_io_threads=4,
                 database_shared_buffers='1GB', measurement_seconds=20)
    manifest = dict(
        status='running', experiment='generation-size', generations=GENERATIONS,
        workers=WORKERS, batch_size=BATCH, cpu_iterations_per_sample=ITERATIONS,
        modes=['materialized', 'training'], repetitions=2, duration_seconds=20,
        sampler_cpus=sampler, database_cpus=database, evaluator_cpus=evaluators,
        initial_core_loads={c:loads[c] for c in cpus}, suite=suite,
        binary_sha256=bench.file_hash(binary), harness_files=hashes,
        driver_sha256=bench.file_hash(Path(__file__)),
        scope='Only generation size changes within each feedback mode. One fixed CPU arithmetic workload, no sleeps. Fixed core allocations for all trials; fresh database per repeat. Paired direct/production reference uses the same generation size and workload. Both case order and measurement method order reverse on repeat 2. Shared host; feedback transports/ingests values without model training.',
        **bench.machine_metadata(),
    )
    bench.write_json(output / 'manifest.json', manifest)
    records = []
    started = time.monotonic()
    try:
        for repeat in range(2):
            directory = output / f'repeat-{repeat}'
            directory.mkdir()
            generations = GENERATIONS if repeat == 0 else GENERATIONS[::-1]
            modes = ['materialized', 'training'] if repeat == 0 else ['training', 'materialized']
            with bench.Session(binary, directory, 900, 99,
                               infrastructure_cpus=database, shared_buffers='1GB') as session:
                session.start_pinned_workers(evaluators, sampler)
                for generation in generations:
                    case_suite = dict(suite, generation_batch_size=generation)
                    for mode in modes:
                        case = directory / f'generation-{generation}-{mode}'
                        case.mkdir()
                        (case / 'window').mkdir()
                        point = frontier.Point(mode, 5.0, WORKERS, BATCH)
                        card = frontier.card(mode, 0, case_suite).replace(
                            'cpu_iterations_per_sample = 0',
                            f'cpu_iterations_per_sample = {ITERATIONS}',
                        )
                        native = dict(batch_size=BATCH, generation_size=generation,
                                      cpu_iterations_per_sample=ITERATIONS,
                                      feedback=mode=='training', duration_seconds=20,
                                      evaluator_cpus=evaluators, sampler_cpus=sampler)
                        results = {}
                        order = ['direct', 'gammaboard'] if repeat == 0 else ['gammaboard', 'direct']
                        for role in order:
                            if role == 'direct':
                                results[role] = amortization.run_direct(binary, case / role, native)
                            else:
                                with frontier.LiveRun(session, case / role, case_suite, mode, 5.0,
                                                      point, card_text=card) as live:
                                    results[role] = live.measure(point, case / 'window')
                        measured = json.loads((case / 'window/measurement.json').read_text())
                        snapshots = [frontier.runtime(s) for s in measured['snapshots'][1:]]
                        row = dict(
                            repeat=repeat, generation=generation, mode=mode,
                            workers=WORKERS, batch=BATCH, directory=str(case.relative_to(output)),
                            runner=results['gammaboard'],
                            activity=bench.activity(measured),
                            largest_production_step_ms=max(
                                s['sampler']['produce_ms']['max'] or 0 for s in snapshots),
                            load_average=os.getloadavg(),
                            **amortization.comparison(results['direct'], results['gammaboard']),
                        )
                        records.append(row)
                        bench.write_json(output / 'results.json', records)
                        print(f"{len(records)}/12: repeat={repeat} G={generation} {mode}, "
                              f"production={row['gammaboard_samples_per_second']/1e6:.3f} M/s, "
                              f"direct={row['direct_samples_per_second']/1e6:.3f} M/s, "
                              f"overhead={row['overhead_percent']:.2f}%, "
                              f"evaluator compute={row['activity']['evaluator_compute']:.1f}%, "
                              f"production step max={row['largest_production_step_ms']:.1f} ms", flush=True)
        manifest['status'] = 'completed'
    except BaseException:
        manifest['status'] = 'incomplete'
        raise
    finally:
        manifest['elapsed_seconds'] = time.monotonic() - started
        bench.write_json(output / 'manifest.json', manifest)


if __name__ == '__main__':
    main()
