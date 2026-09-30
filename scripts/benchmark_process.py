"""Targeted production process API tests and paired callback/adapter timings."""
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

import benchmark_common as bench


def execute(args):
    output = args.output.absolute()
    if output.exists():
        raise ValueError(f'output already exists: {output}')
    output.parent.mkdir(parents=True, exist_ok=True)
    python = str(args.python.absolute()) if args.python else shutil.which('python3')
    if not python:
        raise ValueError('pass --python with a Python interpreter containing NumPy')
    build = subprocess.run(['cargo', 'test', '--locked', '--profile', 'dev-optim',
                            '--test', 'process_api', '--no-run', '--message-format=json'],
                           cwd=bench.ROOT, capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    executable = next(Path(a['executable']) for a in artifacts if a.get('reason') == 'compiler-artifact'
                      and a['target']['name'] == 'process_api' and a.get('executable'))
    cpus = bench.idle_cpus(bench.physical_cpus(), 2)
    if len(cpus) != 2:
        raise ValueError('process measurements require two physical cores')
    environment = dict(os.environ, GAMMABOARD_PROCESS_PYTHON=python,
                       GAMMABOARD_PROCESS_OUTPUT=str(output), GAMMABOARD_PROCESS_CHILD_CPU=str(cpus[1]))
    start = time.monotonic()
    result = subprocess.run(['taskset', '-c', str(cpus[0]), str(executable), '--ignored',
                             '--nocapture', '--test-threads=1'], cwd=bench.ROOT,
                            env=environment, capture_output=True, text=True, timeout=300)
    output.mkdir(exist_ok=True)
    (output/'tests.log').write_text(result.stdout + result.stderr)
    measurement_path = output/'measurements.json'
    batches = ({row['batch'] for row in json.loads(measurement_path.read_text())['rows']}
               if measurement_path.exists() else set())
    manifest = dict(measured_at=datetime.now(timezone.utc).isoformat(),
                    batch_sizes=sorted(batches),
                    experiment='process_api', schema_version=1, cpus=cpus,
                    elapsed_seconds=time.monotonic()-start, load_average=os.getloadavg(),
                    python=python, binary_sha256=bench.file_hash(executable),
                    profile='dev-optim (opt-level=2)', status='passed' if result.returncode == 0 else 'failed')
    sources = output/'sources'; sources.mkdir()
    for path in [bench.ROOT/'tests/process_api.rs', Path(__file__),
                 bench.ROOT/'process_api/python/tests/runtime_fixture.py']:
        shutil.copy2(path, sources/path.name)
    shutil.copytree(bench.ROOT/'process_api/python/src/gammaboard_process', sources/'gammaboard_process',
                    ignore=shutil.ignore_patterns('__pycache__'))
    manifest['source_hashes'] = {str(p.relative_to(sources)):bench.file_hash(p)
                               for p in sources.rglob('*') if p.is_file()}
    bench.write_json(output/'manifest.json',manifest)
    result.check_returncode()
    report(output, plots=False)
    return output


def summarize(measurement):
    rows = []
    for raw in measurement['rows']:
        wall, callback = raw['wall_seconds'], raw['callback_seconds']
        if not wall or len(wall) != len(callback) or any(not math.isfinite(w) or not math.isfinite(c) or c < 0 or w < c for w,c in zip(wall,callback)):
            raise ValueError('invalid paired process timings')
        overhead = [w-c for w,c in zip(wall,callback)]
        rows.append(dict(operation=raw['operation'], batch=raw['batch'], work=raw['work'],
                         feedback=raw['feedback'], calls=len(wall),
                         wall_us=statistics.mean(wall)*1e6, callback_us=statistics.mean(callback)*1e6,
                         overhead_us=statistics.mean(overhead)*1e6,
                         overhead_median_us=statistics.median(overhead)*1e6,
                         overhead_fraction=sum(overhead)/sum(wall)))
    return rows


def report(directory, plots=True):
    measurement = json.loads((directory/'measurements.json').read_text())
    manifest = json.loads((directory/'manifest.json').read_text())
    rows = summarize(measurement)
    bench.write_json(directory/'summary.json',rows)
    lines = ['# Process API overhead', '', measurement['scope'], '',
             f"Tests: {manifest['status']}; {manifest['elapsed_seconds']:.1f}s excluding compilation; "
             f"parent/child physical cores: {manifest['cpus']}; {manifest['profile']}.", '',
             'Three warmup calls are discarded. Means use matched native and callback wall times from the same calls. '
             'The residual includes SDK validation/packing, pipes, native conversion/accumulation, deallocation and scheduling; '
             'it is not a pure IPC latency. Input construction and process startup are excluded; startup and every timing '
             'are retained in `measurements.json`. Work is 0 or 64 in-place NumPy sine passes, not a nominal microsecond target.', '',
             '| Operation | Batch | Work passes | Feedback | Callback µs/sample | Residual µs/sample | Residual share |',
             '| --- | ---: | ---: | --- | ---: | ---: | ---: |']
    for r in rows:
        if not r['batch']: continue
        lines.append(f"| {r['operation']} | {r['batch']:,} | {r['work']} | {r['feedback']} | "
                     f"{r['callback_us']/r['batch']:.4f} | {r['overhead_us']/r['batch']:.4f} | {r['overhead_fraction']:.1%} |")
    lines += ['', 'The functional test also covers six continuous dimensions plus a discrete dimension, '
              'non-unit weights, feedback on/off, variable batch sizes, ingestion totals, invalid requests '
              'and subsequent worker reuse. These targeted measurements do not model GPUs, model initialization '
              'or arbitrary user callbacks. No performance threshold is asserted on this shared host.', '']
    lines += ['![Sampler and evaluator overhead versus batch size](overhead.png)', '']
    (directory/'report.md').write_text('\n'.join(lines))
    if plots:
        import matplotlib
        matplotlib.use('Agg')
        import matplotlib.pyplot as plt
        fig, axes = plt.subplots(2, 2, figsize=(13, 8), sharex='col')
        styles = [
            [('generate', True, 'Generation', '#2563eb'), ('feedback', True, 'Training feedback', '#c2410c')],
            [('eval', False, 'No training feedback', '#2563eb'), ('eval', True, 'With training feedback', '#c2410c')],
        ]
        for column, series in enumerate(styles):
            for operation, feedback, label, color in series:
                for work, linestyle in [(0, '-'), (64, '--')]:
                    group = sorted((r for r in rows if r['operation'] == operation and
                                    r['work'] == work and r['feedback'] == feedback), key=lambda r:r['batch'])
                    if not group:
                        continue
                    name = f'{label} · {work} work passes'
                    sizes = [r['batch'] for r in group]
                    axes[0, column].plot(sizes, [r['overhead_us']/1000 for r in group],
                                         marker='o', linestyle=linestyle, color=color, label=name)
                    axes[1, column].plot(sizes, [r['overhead_us']/r['batch'] for r in group],
                                         marker='o', linestyle=linestyle, color=color)
            axes[0, column].set_title(['Sampler', 'Evaluator'][column])
            axes[0, column].legend(fontsize=8)
            axes[1, column].set_xlabel('Samples per batch (log scale)')
            for ax in axes[:, column]:
                ax.set_xscale('log'); ax.set_yscale('log')
                ax.grid(alpha=.2, which='both')
        axes[0, 0].set_ylabel('Overhead per call (ms, log scale)')
        axes[1, 0].set_ylabel('Overhead per sample (µs, log scale)')
        fig.suptitle('Process protocol overhead vs batch size\nRust ↔ Python SDK · mean paired wall time minus callback time', fontsize=13)
        fig.text(.5, .015, 'Includes packing, validation, IPC and native conversion/accumulation. Shared host; excludes startup and database.',
                 ha='center', fontsize=9)
        fig.tight_layout(rect=(0, .035, 1, .94))
        for ext in ['png','svg','pdf']: fig.savefig(directory/f'overhead.{ext}',dpi=180)
        plt.close(fig)
    return directory/'report.md'
