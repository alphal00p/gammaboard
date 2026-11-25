# GammaLoop through the Rust process API

This executable wraps GammaBoard's native GammaLoop evaluator with the existing
stdio protocol. The native and external paths use the same pinned GammaLoop,
state loading, preprocessing, parameterization, precision settings and numerical
evaluation code. One initialized evaluator is retained for the process lifetime.
There is no database or server inside the wrapper.

The example currently links the GammaBoard library to reuse that adapter. It is
an explicit Cargo example, not a dependency of a GammaLoop-free host. Extracting
the adapter into its own crate can happen alongside the future observable-model
decoupling, without changing this wire interface.

## Build and inspect

From the repository root, in the same build environment as native GammaBoard:

```bash
cargo build --release --locked --example gammaloop_process
```

The binary is `target/release/examples/gammaloop_process`. Copy it to an explicit
path accessible to every evaluation worker. It needs the same runtime libraries,
Symbolica licensing setup and generated GammaLoop state as the native evaluator.
This example uses Unix descriptor redirection: native stdout is sent to stderr,
leaving the protocol stream clean even during native compilation and warm-up.

For the bundled gg → HHH run cards, generate a fresh state if necessary:

```bash
cargo run --release --locked --example prepare_gammaloop -- /absolute/path/gg_hhh
```

Copy `args.json` and set `state_folder` to that absolute path on the worker host.
All other fields follow the native GammaLoop evaluator parameters, including
`process_id`, `integrand_name`, `graph_groups`, `momentum_space`, `use_f128`,
`reference_gaussian` and `preprocessing`. Preprocessing defaults to read-only.

```bash
/absolute/path/gammaloop_process --describe /absolute/path/args.json
```

This loads and warms the evaluator, then prints `domain`, `descriptor`, and
`metadata`. The run card can omit `domain` and `output`: GammaBoard discovers the
contract at validation and caches it. Explicit `domain`/`descriptor` assertions are
also supported. An explicit descriptor is useful when the service host cannot run
the worker executable; see [discovery deployment details](../../protocol.md#selected-output-evaluator-contract).
Discrete and ragged domains are supported.

The JSON file is only an input to `--describe` and the acceptance tests. During a
run, GammaBoard sends the run card's `[evaluator.args]` over the protocol.

## Run without GammaLoop in the host

Edit `run.toml` to set the absolute executable and state paths. It returns
`[real, imag]` values and uses uniform sampling with ordinary vector accumulation:

```bash
./gb run create process_api/examples/rust_gammaloop/run.toml --start
```

An already running instance and available workers are required. This card works
when both the service and worker host are built with `--no-default-features`:

```bash
cargo build --release --locked --no-default-features --bin gammaboard \
  --target-dir target/minimal
```

Use `target/minimal/release/gammaboard` for that instance's service and worker
processes; the separately built wrapper still contains GammaLoop. No process
protocol change or Python SDK is needed. The host applies sampling weights,
stratification and statistical reduction. The wrapper receives unit-weight
coordinates and returns the integrand including the parameterization Jacobian.

For a scalar projection, request `value` with a scalar accumulator and set `args.training_projection` to `real`, `imag`, `abs` or `abs_sq`, with a
matching scalar accumulator on the host.

## Observable output

To collect histogram bundles as well as complex values, select the task accumulator:

```toml
# In [[task_queue]]:
accumulator = { config = "gammaloop" }
```

This mode still requires the host's `gammaloop` feature because the current
observable model and accumulator use GammaLoop types. Decoupling those types is
future work. Each response contains one unweighted snapshot per original sample,
including empty fills; the host performs the reduction. Stability counters, timings
and extreme-point evaluation details are transported by the selected-output contract.

The wrapper now advertises compact binary observable transport. Supporting
hosts receive the schema once, followed by per-sample sparse numeric bin data.
Older hosts automatically receive JSON bundles instead. To force the compatible
JSON path (including for layouts that change during evaluation), append
`--observable-format json` to the worker command. Both modes serialize typed
responses directly; the host no longer clones intermediate JSON result trees.
See the [selected-output wire contract](../../protocol.md#selected-output-evaluator-contract).

## Compare the execution paths

For an automated real-instance benchmark with **observable training and
inference**, use `python -m benchmarks.gammaloop_ab`. It calibrates fixed sample
budgets, starts fresh instances, alternates backend order, and writes an HTML
report with raw results. See the [benchmark instructions](../../../docs/benchmarking.md#choose-a-measurement).

`run.toml` and `run-native.toml` have matching seeds, sampling budgets, queue sizes,
GammaLoop settings and accumulation. Change the state path in both cards and the
wrapper path in the process card. Run them sequentially on the same workers:

```bash
./gb run create process_api/examples/rust_gammaloop/run-native.toml --start
./gb run wait gammaloop-native --until completed
./gb run create process_api/examples/rust_gammaloop/run.toml --start
./gb run wait gammaloop-process --until completed
```

Use a release build for both executable paths. Keep worker count, CPU allocation,
batch sizes and state identical. Compare task results and worker throughput;
separate process startup/state loading from sustained evaluation. Repeat with
different `fixed_batch_size` values to see IPC amortization. Both cards use
values-only output and the same native per-sample evaluation path. Observable
construction is skipped when not requested. For a comparison including observable
transport, diagnostics and reduction, enable the GammaLoop accumulator on both cards.

First compare the two paths with the same full-featured host. Then repeat the
process card with the minimal host to measure the separate build configuration.
The minimal host cannot run the native card. These cards provide a reproducible
comparison workload, not a claim about the size of the performance difference.

## Acceptance checks

Build the wrapper first, then point the tests at it and an explicit parameter file:

```bash
export GAMMABOARD_TEST_GAMMALOOP_PROCESS=/absolute/path/gammaloop_process
export GAMMABOARD_TEST_GAMMALOOP_ARGS=/absolute/path/args.json
cargo test --locked --example gammaloop_process
cargo test --locked --test rust_gammaloop_process -- --ignored
cargo test --locked --no-default-features --test rust_gammaloop_process -- --ignored
```

The full-featured checks compare native and process scalar/complex values and
per-sample observable bundles, then compare weighted histogram reduction. The
minimal-host check exercises the actual process adapter, repeated/empty batches,
recovery after an invalid batch, clean metadata discovery and host-side weights.
No PostgreSQL instance is required. Use a nonzero saved integrand; a state with
histograms exercises the observable checks most fully.

## Unified requests and diagnostics

The wrapper advertises `gammaboard-evaluation-v1` and accepts per-call component,
observable, raw-evaluation-detail and diagnostic selections. One configuration can
serve training, inference, images and lines. Values already include the mapping
Jacobian; raw integrand/Jacobian details are informational. Process diagnostics now
include the same counters and timings as native evaluation. JSON and the negotiated
compact encoding carry the same data; `--observable-format json` forces JSON for
comparison. Older hosts still use the retained v10 `eval_batch` adapter.

The observable types currently enter GammaBoard through `evaluation::observables`.
Their future extraction into a separate crate changes that import boundary, not the
selected-output contract. Observable reduction still requires the GammaLoop feature
until that extraction; value-only process evaluation does not.
