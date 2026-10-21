# Benchmarks

One CLI produces separate PNG/SVG plots, CSV summaries, raw JSON, and a local
HTML index. Feedback off/on is paired by default. Use Linux, an optimized binary,
PostgreSQL tools, `taskset`, and Python with NumPy/matplotlib. The suite owns a
private database; no dashboard or running deployment is needed.

```bash
cargo build --locked --profile dev-optim --bin gammaboard
python3 -m venv .venv
.venv/bin/pip install numpy matplotlib
.venv/bin/python -m benchmarks all --quick --output results/quick
```

Open `results/quick/report.html`, or individual plots directly. Reports include
failed/unmeasured coverage rather than converting failures to zero throughput.
Use `--binary PATH` for another executable and `--python PATH` for its process
worker interpreter. `CARGO_TARGET_DIR` is honored. `just benchmark` is shorthand.

## Choose a measurement

| Command | What it measures |
| --- | --- |
| `plan` | Planned cases and measurement budget |
| `frontier` | Accepted throughput through the actual sampler/evaluator pipeline |
| `sampler-io` | Inserts and result collection with lightweight consumers |
| `evaluator-io` | Claim/decode/submit against a prefilled queue |
| `protocol` | Rust/Python adapter time minus callback time; no database |
| `amortization` | Extra steady-state runtime versus a matched in-memory pipeline |
| `all` | Frontier, both I/O sweeps, and protocol; amortization is separate |

```bash
python -m benchmarks frontier --output results/frontier
python -m benchmarks sampler-io --io-threads 1 4 --output results/sampler
python -m benchmarks evaluator-io --output results/evaluator
python -m benchmarks protocol --batch-sizes 16 1024 32768 131072 --output results/protocol
python -m benchmarks amortization --workers 1 16 --target-batch-seconds 0.16384 --output results/amortization
```

`--quick` reduces counts/repetitions; `--max-evaluators`, `--cpu-limit`,
`--memory-mib`, `--duration`, and `--repetitions` bound resources/runtime.
`--memory-mib` covers estimated I/O payloads, not PostgreSQL cache or worker memory.
CPU groups are disjoint but the host is not reserved exclusively. Use `--help`
for targeted overrides; explicit settings must fit the machine.

## Interpret results

**Frontier:** zero-delay evaluator doublings from 1 to 512, with sparse 5 µs,
200 µs, and 5 ms curves. Delays use batch sleeps with seeded 10% jitter, not CPU
work. Default modes transmit six-dimensional materialized samples with feedback
off/on; `--include-rng` adds compact inference. Batch defaults and exact points
are in [frontier.toml](../benchmarks/frontier.toml). These are selected scaling
curves, not an optimization search over every configuration. Readiness, stable
coverage, complete generation intervals, and accepted progress determine validity.

**I/O:** warmed database-path capacities, including serialization, with sampler
arithmetic or evaluator arithmetic bypassed. Default batches span 256–131,072;
sampler threads default to 1, 2, 4, 8 within the CPU budget. Reports include batches/s
and encoded MiB/s (excluding WAL/network framing). I/O busy at or below 90% prints
an unsaturated warning. High busy includes database waits; it does not prove
hardware saturation. `--profile` records database waits for diagnosis.

**Protocol:** production adapters and writable callback arrays, including packing,
validation, conversion/accumulation, cleanup, pipes, and scheduling. Subtracts
paired callback time; this is adapter overhead, not pure IPC latency. Sampler
measurements retain native generations; training includes feedback. Ordinary
evaluators return values in both modes, so similar curves are expected.

**Amortization:** real calibrated CPU work, paired with a direct in-memory reference.
Defaults: 1/16 evaluators, five batches from 128 to 32,768, three repeats, feedback
off/on. Requires 25 physical cores at default allocation; reduce workers on smaller
machines. `--target-batch-seconds` varies work/sample with batch size, plotting
measured cost/sample; otherwise cost/sample stays fixed. Extra runtime is
`100 × (direct_rate / gammaboard_rate − 1)`; negative observations are retained.
Warmup/draining are excluded. This excludes adaptive-model optimization, GPUs,
large histogram payloads, and multi-host networking.

Large batches can reduce throughput through memory/copy/database pressure; queue
limits may also change under a memory budget. Start with the production **32,768**
transport cap and **262,144** generation cap, then remeasure the intended workload.
Compare matching CPU allocations, batch/generation settings, and cache conditions;
protocol or isolated I/O improvements alone do not establish end-to-end gains.

[Saved measurements and plots](benchmarks/) retain their original settings and
source provenance. Use current runs to assess current code. Benchmark definitions
live in [benchmarks/](../benchmarks/) and [src/benchmark/](../src/benchmark/).
