# External process workers

This is the setup and implementation guide for external evaluators, samplers,
materializers and transforms. Commands below run from the repository root. Install
the SDK into the worker environment from the same revision as GammaBoard:

```bash
python -m pip install ./process_api/python
```

| Reference | Contents |
| --- | --- |
| [Integration guide and evaluator checker](integration.md) | Weight ownership, discrete measure, component order, padding, constants and conformance checks |
| [Execution model](../docs/execution-model.md) | Full sampler → materializer → transforms → evaluator → feedback contract, ownership, weights, budgets, and recovery |
| [Wire protocol](protocol.md) | Current JSON-RPC framing, every role/method, binary arrays, and errors |
| [Python interfaces](python/src/gammaboard_process/abc.py) | Typed callbacks and their defaults |
| [Batch types](python/src/gammaboard_process/batches.py) | `Batch`, `SampleBatch`, `WAITING`, and `FINISHED` |

## Configure a worker

```toml
[evaluator]
kind = "process_evaluator"
command = ["/path/to/venv/bin/python", "-m", "my_worker"]
cwd = "/shared/project"
args = { scale = 2.0 }
```

All process roles share `command`, optional `cwd`, `args`, and
`shutdown_grace_seconds` (30 by default). Process samplers inherit the run domain.
Evaluators declare their domain and available channels. Tasks select components
through their accumulators, including `gammaloop` for observable integration.
Optional explicit `domain`/`descriptor` assertions support deployment validation.
Commands may launch a local executable or an explicit container invocation.

## Entrypoints

A worker module imports its class and calls the matching runner:

```python
from gammaboard_process import run_evaluator
from my_evaluator import MyEvaluator

if __name__ == "__main__":
    run_evaluator(MyEvaluator)
```

Use `run_sampler`, `run_materializer`, or `run_batch_transform` for the other roles.
The SDK owns framing and NumPy conversion; subclasses of the SDK base classes are
optional. The callback contract is described below; the [execution model](../docs/execution-model.md)
defines behavior shared with native implementations.

## Python callbacks

Matching methods are sufficient; SDK base classes are optional. Fresh evaluator,
materializer and transform classes are constructed as
`ClassName(domain=domain, **args)`, using the role-specific typed `Domain`.
Samplers are constructed as `ClassName(domain=domain, evaluator_metadata=metadata, **args)`.
Pass a class to `run_*`, not an already constructed instance. `args` cannot define
the reserved keys `domain` or `evaluator_metadata`.

| Role | Python callback | Result |
| --- | --- | --- |
| Evaluator discovery | `descriptor()` | `EvaluatorDescriptor(domain, components, observables=False, details=False, diagnostics=False, scalar_projection=None)` |
| Evaluator | `evaluate(xs_discrete, xs_continuous, request)` | `EvaluationBatch(values, observables=None, details=None, diagnostics=None)` |
| Sampler | `generate(max_samples)` | `SampleBatch(xs_discrete, xs_continuous, weights, training_remaining=None)` or `WAITING` / `FINISHED` |
| Sampler feedback | `feedback(feedback)` | Complete draw with masked `.values`, `.unmasked_values`, and optional `.strata` |
| Materializer | `materialize_batch(latent_batch)` | `Batch(xs_discrete, xs_continuous, weights)` |
| Transform | `transform_batch(xs_discrete, xs_continuous, weights)` | The same `Batch` type, with replacement total weights |

Fixed-width discrete and continuous coordinates are NumPy arrays with shapes
`(N,D_d)` and `(N,D_c)` respectively; `C` in the output table is the evaluator's
component count. Variable-width coordinates are tuples of one-dimensional arrays. Weights and
feedback value arrays have shape `(N,)`. SDK callback input arrays own writable
storage and can be retained without corrupting later requests. All output arrays
must have the required shape and finite values. Values may be an `(N,C)` array in
`request.components` order, `(N,)` for one component, or a mapping keyed by component
name. Optional channels are lists of N per-sample dictionaries, computed only when
requested. `None` means unavailable. `request` has `components`, `observables`,
`details` and `diagnostics` fields.

Evaluator construction receives `domain=None` during automatic discovery. The
provider derives its domain from its arguments/state and returns it in the descriptor.
A supplied domain is an assertion. New run cards can omit both `domain` and `output`;
select integration/plot outputs through the task accumulator. Explicit descriptors
avoid starting a probe on the service host. See the [selected-output contract](protocol.md#selected-output-evaluator-contract)
for validation, binary encoding, sidecar fields and discovery deployment constraints.

Legacy classes implementing only `eval(xs_discrete, xs_continuous)` remain supported
with explicit `domain` and `output`. `ObservableBatch` is a compatibility alias for
`EvaluationBatch`. The preferred interface has one evaluation hook for every output.

Samplers/materializers receive the augmented domain, including stratum prefixes;
evaluators/transforms receive the integrand domain. Use
`domain.require_rectangular_shape()` only if the algorithm needs a rectangular
layout; otherwise inspect branching nodes, axis roles and rectangular `strata_dims`.
See [Domain](../docs/execution-model.md#samples-domains-and-queue-payloads).

`feedback` receives one original draw in generation order, even when evaluation
was split across workers. Both `.values` and `.unmasked_values` are weighted;
`.values` additionally masks stratum mismatches. Optional `.strata.requested` and
`.strata.actual` are prefix tuples. The sampler adapts its proposal and never
aggregates the reported integral or observables.

### Snapshots and stage handoff

Every sampler class must implement `snapshot()` and this restoration entry point:

```python
@classmethod
def from_snapshot(cls, *, snapshot, domain,
                  init_args, evaluator_metadata=None):
    ...  # Return a fully restored sampler instance.
```

`snapshot()` returns non-null, JSON-safe state (use `{}` for empty state), including
private RNG/adaptation state and draws waiting for feedback. `init_args` is the
construction dictionary, not expanded keywords on restoration. To transfer a
trained proposal into a fresh stage, optionally implement:

```python
@classmethod
def from_stage(cls, *, source_state, source_args, init_args,
               domain, evaluator_metadata=None):
    ...  # Return a fresh instance initialized from the trained proposal.
```

This is not recovery: reset stage-local progress and pending feedback. If it is
not implemented, handoff fails explicitly. For a successful database save, the
optional notification hook is:

```python
def checkpoint_committed(self, *, snapshot) -> None:
    ...  # Mark this exact artifact/state as committed; cleanup follows retention policy.
```

The SDK ABC provides a no-op default; duck-typed samplers may omit it entirely.
The payload is exactly the opaque value returned by `snapshot()`. Delivery is
best effort and may repeat, so the hook must be idempotent and `from_snapshot`
must also establish committed state. It does not authorize deleting artifacts
still referenced by stages or backups. The [recovery contract](../docs/execution-model.md#tasks-checkpoints-and-failures)
defines save ordering, retries and notification supersession.

### Optional queries

Evaluators may implement `metadata()` (default `{}`), whose JSON value is forwarded
to sampler initialization, and `diagnostics()` (default `{}`). For samplers,
`diagnostics()` returns a JSON object (default `{}`); `persisted_output()` returns
optional display/task output (default `None`), separate from recovery state.
Sampler probability queries are `pdf(xs_discrete, xs_continuous)` and `discrete_mass(subspaces)`; the latter receives
a list of dictionaries mapping discrete dimension indices to fixed values. Both
return a sequence or one-dimensional NumPy array, with one entry per query in the
same order, or `None` for a wholly unavailable response. Entries may independently
be `None`; known zero is `0.0`. PDF values are normalized joint densities and may
exceed one. Discrete masses marginalize unspecified discrete dimensions and
integrate continuous coordinates, and must lie in `[0,1]`. Negative or non-finite
values are errors. There is no global PDF normalization hook.

## Check a sampler

[check_sampler.py](check_sampler.py) exercises a process sampler without a
GammaBoard server or database. The checker environment needs NumPy and the SDK;
`pip install ./process_api/python` installs both. An executable worker still uses
its own interpreter and installed dependencies.

Use the same command, working directory and constructor arguments as the run card:

```bash
python -B process_api/check_sampler.py "/path/to/venv/bin/my-sampler" \
  --init-args '{"seed": 42, "save_path": "/tmp/gammaboard-sampler-check"}'

python -B process_api/check_sampler.py \
  --command '["/path/to/venv/bin/python", "-u", "worker.py"]' \
  --cwd /path/to/worker
```

For development, `module:Class` or `/path/module.py:Class` uses the checker's
interpreter and SDK; imports happen in child processes. Use disposable sampler
output paths: the checker creates no runs, database records or persistent reports,
but your sampler can create or modify its own files.

The required checks cover bounded generation, shutdown, JSON snapshots, recovery
without a commit notice, preservation of a committed snapshot after a failed save,
and pending-feedback recovery for training samplers. Other checks exercise commit
notifications, PDFs/masses, diagnostics, stage handoff and optional toy statistics.
Recovery scenarios use separate sequential worker processes, avoiding simultaneous
copies of a GPU model. Database commit notifications are simulated; PostgreSQL
transaction behavior is not tested here.

| Option | Purpose |
| --- | --- |
| `--init-args JSON`, `--domain JSON`, `--evaluator-metadata JSON` | Match the run's inputs; each also accepts `@file` |
| `--stage-init-args JSON` | Override fresh-stage constructor arguments |
| `--require-stratified` | Require complete stratum-prefix masses; the default domain has two strata |
| `--extra-domains` | Also exercise discrete-only, mixed and ragged domains |
| `--feedback` | Choose `positive`, `zero`, or `signed` toy training feedback |
| `--statistics` | Check PDF/weight consistency and toy integrals for frozen inference, including stratified domains |
| `--max-samples`, `--max-calls`, `--batch-sizes` | Bound the test trajectories; default maximum is 128 samples per trajectory |
| `--timeout`, `--time-limit` | Per-RPC and total time limits; defaults are 60 and 300 seconds |
| `--rtol`, `--atol` | Explicit replay tolerances; defaults require exact replay |

The ordinary toy check integrates `1` on a rectangular unit domain. With strata,
it splits the first continuous coordinate into equal intervals, one per prefix
in domain order, and integrates both `1` and `x[0] - 1/2`. For two strata and no
integrand discrete axes, the signed contributions are `-1/8` and `+1/8`. Multiple
stratum dimensions and ragged prefixes work when every prefix has the same
rectangular integrand domain. Remaining discrete axes use counting measure.

The stratified check queries exact prefix masses `p_k`, masks points outside their
intended interval, retains zeros, and averages `p_k * weight * masked_value` over
each stratum's attempted draws. It checks each contribution and the combined
estimate against the exact integrals; combined variance is the sum of the
per-stratum variances. This exercises probability weighting and prevents signed
cancellation from hiding wrong contributions.

Use frozen inference with continuous coordinates in `[0,1]` and proposal support
covering each intended toy interval. The check needs at least 32 attempts and two
matches **per stratum** (32 total draws without strata). For example:

```bash
python -B process_api/check_sampler.py "/path/to/venv/bin/my-sampler" \
  --require-stratified --statistics --max-samples 1024 --max-calls 128
```

Training draws, unavailable masses, unsupported domains, or insufficient coverage
leave the numerical check inconclusive. A discrepancy beyond six estimated
standard errors also reports inconclusive and needs investigation. These smoke
checks do not certify normalization, independence or statistical coverage.

Unavailable optional operations are **SKIP**; invalid responses are **FAIL**.
**INCONCLUSIVE** means a check could not establish its result and is not a pass.
Exit codes are `0` for required checks passed, `1` for a failed check, `2` for
invalid setup or an inconclusive required check, and `130` for interruption.
Optional inconclusive results do not fail the run. Over RPC the checker cannot
distinguish a successful no-op from a custom optional implementation; class mode
can skip inherited defaults. See `--help` for all options.

## Working examples

- [NumPy evaluator](examples/python_evaluator/README.md): ordinary scalar/vector values.
- [NumPy sampler](examples/python_sampler/README.md): uniform draws, feedback,
  exact PDFs/masses, snapshots and inference, without Symbolica.
- [Synthetic observable evaluator](examples/python_observables/README.md):
  a polynomial and one GammaLoop-format histogram bundle per input draw. Its
  [stratified observable run card](examples/python_observables/run-stratified.toml)
  uses stratified statistics for the integral and histogram bins.
- [Rust/Apptainer evaluator](examples/rust_evaluator/README.md): a direct
  wire implementation, including ragged coordinate layouts.
- [Rust GammaLoop wrapper](examples/rust_gammaloop/README.md): the native
  evaluator behind the process API, with matching native/process run cards and
  a values-only mode usable by hosts built without GammaLoop.

See the [tutorial index and shared setup](examples/README.md). These tutorials are
separate from the default dashboard run catalog.

## Logs and lifecycle

Stdout belongs to the protocol. The SDK redirects `print` to logs; use
`gammaboard_process.log(message, level="info")` for structured levels.
`GAMMABOARD_LOG_LEVEL` filters worker messages; runtime `tracing.db_gammaboard_level`
filters persistence. Direct stderr is recorded at warning level.

Each role runs in a separate subprocess with synchronous RPC. EOF closes a worker;
the host waits the configured grace period before terminating its process group.
A process can be restarted or receive replayed work. Keep numerical behavior
independent of prior evaluator/materializer/transform calls, and include all
sampler-private pending state in snapshots. See [failure and recovery semantics](../docs/execution-model.md#tasks-checkpoints-and-failures).
