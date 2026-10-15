# Upgrade to 0.3

This release changes run configuration, Rust extension types, saved sampler and
accumulator state, queued work, telemetry, and the Python process SDK. It requires
a fresh database. Package version 0.3, process protocol v6, database model 1,
checkpoint format 1, and performance schema 2 identify different contracts;
their version numbers do not have to match.

## Preserve the old deployment

Use the old executable to export the run definitions you need and create a
database backup. Retain that executable, its Python environment, generated physics
states, and external sampler artifacts with the backup. An old database can only
be resumed with its matching old software. Stop all old servers and workers.

Start 0.3 against a new database URL and data directory. For an intentional reset
of a disposable deployment, `gammaboard db reset --yes` deletes its existing data.
The new executable rejects an unversioned or unsupported database before worker
registration. This guard cannot change the behavior of an old executable: never
point old workers at the new database.

## Convert definitions before recreating runs

Install `tomlkit` in a development environment, then generate a separate file:

```bash
python scripts/convert_run.py old-run.toml
python scripts/convert_run.py old-run.toml --output new-run.toml
gammaboard run validate new-run.toml
gammaboard run validate new-run.toml --probe
```

Without `--output`, the tool prints a dry-run result. It never overwrites an
existing file. It converts inline child definitions and leaves referenced files
for separate conversion. It preserves opaque process `args`, replacement values,
and constants. Review the result: the converter cannot rewrite callback code,
invent missing artifacts, or establish the intended meaning of ambiguous input.
It rejects competing sources and ambiguous stopping projections.

| Old form | New form |
| --- | --- |
| Task `sampler_aggregator` | Task `sampler` |
| Symbolica `expr` and `constants` | `source = { kind = "expression", expr = "...", constants = {...} }` |
| Symbolica `compiled_args` and `bindings` | Put them inside `source = { kind = "compiled", ... }` |
| Process evaluator `accumulator` and `components` | `output = { kind = "values", components = [...] }`, or `{ kind = "gammaloop" }` |
| `{ continuous = { dims = 2 } }` | `{ rectangular = { continuous_dims = 2, discrete_cardinalities = [] } }` |
| Measurement `quantity = "central_value"` | `selection = { kind = "all_component_means" }` |
| Measurement `metric = "variance"` | `selection = { kind = "metric", name = "variance" }` |
| Sample `nr_samples` | `stop_condition.max_samples` |
| GammaLoop stop `projection = "abs"` | `metric = { name = "mean", component = "norm" }` |
| GammaLoop stop `projection = "real"` or `"imag"` | `metric = { name = "mean", component = "real" }` or `"imag"` |

The `norm` component is the mean of per-sample complex magnitudes. It is not the
magnitude of the complex mean. An omitted stopping metric retains that norm for
GammaLoop and the training projection for statistical vectors.

Sampling configuration contains the algorithm, optional materializer, and optional
transforms, for example:

```toml
sampler = { config = { kind = "naive_monte_carlo", seed = 7, transforms = [] } }
```

The Rust model separates `SamplingConfig` from its `SamplerConfig` algorithm.
Task `batch_transforms` remains an explicit stage override, including when the
sampler is inherited; otherwise pipeline transforms or the preceding stage apply.
Omitted transforms inherit; an empty list clears them. The worker role remains
`sampler_aggregator` because it executes both sampling and aggregation.

Controller definitions are top-level run kinds, not integration tasks. Parameter
sources must select exactly one of `values`, `linspace`, and `range`; optimizer
configuration is a tagged variant. Definitions predating those changes need
manual restructuring using the current templates. Unknown task and queue options
are errors, including retired spellings and typos.

## Rebuild process workers

Install the SDK and integration from the same checkout until publishing 0.3:

```bash
python -m pip install ./process_api/python ./integrations/madgraph
# In the environment with the required MadNIS dependencies:
python -m pip install ./process_api/python ./integrations/madnis
python -m pip check
```

Integrations require `gammaboard-process>=0.3.0,<0.4.0`. Remove any stale editable
install pointing at a different checkout. Container images and remote worker
environments must be rebuilt too. The sibling glNIS repository needs its 0.2
adapter update and the 0.3 SDK.

Build release wheels from a clean checkout. Ignored `build/` directories reused
across revisions can retain outdated Python modules despite a new package version;
verify the installed exports and protocol, not only package metadata.

Samplers implement `generate(max_samples)`, `feedback(values)`, `snapshot()`, and
`from_snapshot(...)`. `generate` returns `SampleBatch`, `WAITING`, or `FINISHED`;
`training_remaining` declares the feedback window before that draw. Feedback
contains one weighted value per sample of one draw, in order. Restoration includes
pending feedback and RNG state. `diagnostics()` replaces `get_diagnostics()`.

Use `Batch` for transform/materializer outputs. GammaLoop callbacks implement
`eval_gammaloop(points..., weights, require_training_values=...)`, apply weights
exactly once, and return `GammaLoopBatchResult`. See the [wire contract](../process_api/protocol.md)
for precise initialization and array layouts. Old protocol workers are rejected.

## Validate the new deployment

Run installation smoke, a training/inference run, pause/resume, and a worker
replacement before expensive production work. Verify measurement components,
stopping targets, transformed coordinates, and training feedback. Performance
consumers must accept schema 2 and read sampler identity from `metrics.epoch` and
`metrics.node_uuid`, replacing `runtime_metrics.runner_epoch`.

Old queues, checkpoints, and numerical results are not converted. New checkpoint
and queue readers reject unversioned/future data. Backups include database state,
not referenced files; keep external snapshot artifacts immutable for as long as
any saved stage or checkpoint uses them.
