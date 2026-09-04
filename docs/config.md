# Config Reference

This document covers the operator-facing config shapes. Checked-in templates live under `resources/templates` and profile-specific ops configs live under `ops/*/config`.

## Layout

- `src/config_defaults/*.toml`: built-in fallback defaults embedded into the Rust binary.
- Local development uses the embedded runtime/server defaults.
- `ops/itphlies/config`: ITPhlies profiles.
- `ops/ubelix/config/*`: UBELIX profiles rendered/submitted by the UBELIX launcher.
- `resources/templates/{runs,tasks,nodes}/`: shared templates served by the backend.

All commands use the embedded runtime default unless `--runtime-config` points
at a custom file:

```bash
gammaboard --runtime-config path/to/runtime.toml <COMMAND>
```

For common deployment differences, prefer CLI overrides over a nearly-default
runtime file:

```bash
gammaboard \
  --database-url postgresql://... \
  --resource-root /shared/gammaboard/resources \
  <COMMAND>
```

Use `--postgres-public` only when workers on other machines or Slurm nodes must
connect to the local Postgres instance. The embedded PostgreSQL URL uses a fixed
public development credential for the loopback-bound managed database.
`--postgres-public` preserves passwordless remote operation but prints a warning
that PostgreSQL is exposed with `trust` authentication.

Relative evaluator resources, such as GammaLoop `state_folder`, resolve through `resources.roots` in order. Absolute paths are used as-is.

## Server Config

Server config owns both the backend API settings and the foreground deploy
settings used by `gammaboard deploy`:

```toml
api_host = "127.0.0.1"
api_port = 4000
allowed_origins = ["http://localhost:8080"]
secure_cookie = false
allow_local_node_spawn = true

[frontend]
build_dir = "../../../dashboard/build"
host = "127.0.0.1"
port = 8080

[database]
ensure_started = true

# Optional. Omit this section for passwordless local control.
[auth]
admin_password_hash = "..."
session_secret = "..."
```

`gammaboard deploy` validates the frontend build, optionally starts local Postgres, starts the backend, waits for its health endpoint, starts nginx, and verifies the proxied health endpoint before reporting success. It runs nginx in the foreground and on `Ctrl-C`/`SIGTERM` requests graceful node shutdown before stopping nginx, backend, and local Postgres.

## Runtime Config

Runtime config owns the database URL, resource roots, tracing, and local Postgres settings. The checked-in default Postgres port is `5400`; global `--port-offset <N>` shifts frontend/API/Postgres ports and local Postgres state paths for all child processes.

## Auth And Templates

- When `[auth]` is configured, every dashboard API endpoint except health and
  session/login/logout requires the single admin session. There are no user
  accounts or roles.
- Omit `[auth]` only for passwordless trusted-network control. Anyone who can
  access the deployment can create runs and launch their configured process
  commands.
- Put `auth.admin_password_hash` and `auth.session_secret` in your server config to enable dashboard auth.
- Set `allowed_origins` in your server config if the frontend is served from origins other than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in your server config.
- An open or HTTP dashboard is appropriate only on a trusted network; otherwise
  the server prints a startup warning. Keep authentication secrets in an
  untracked private config.

Authentication sessions include expiry, issued-at, issuer, audience, and a
`session_version`. Increase `session_version` to invalidate existing sessions.

Generate the password hash with:

```bash
gammaboard auth hash-password
```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

When `allow_local_node_spawn = true`, the server resolves node launch requests by spawning local child processes. Otherwise launch requests remain queued for an external launcher. External launchers should use the node-launch-request API; `starting` means workers were submitted, and `running` is reconciled from live node leases.

The create-run, add-task, and node-request dialogs can load `.toml` templates from `run_templates_dir`, `task_templates_dir`, and `node_templates_dir`; admin users can also save edited TOML back as templates and delete templates from the dashboard.

## Template Replacements

Run configs, task-append configs, and node-launch configs support an optional top-level `replacements` table. This is intentionally not supported for server or runtime config.

Placeholders use `$(name:default)`. When the complete TOML string value is exactly one placeholder, the replacement value, or the default when no replacement is provided, is parsed as standalone TOML before the final config is deserialized:

```toml
replacements = { run_name = "scan-a", samples = 100_000, mode = "auto" }

name = '$(run_name:"fallback")'
stop_condition = { max_samples = "$(samples:10_000)" }
args = { subtraction_mode = '$(mode:"none")' }
```

If a placeholder appears inside a larger TOML string, it is interpolated as text instead:

```toml
expr = "1 / ((x - $(mu:0.5))^2 + $(width:0.1))"
label = "mode=$(mode:auto)"
```

Defaults are parsed as TOML when possible, otherwise they are treated as raw strings. Numeric, boolean, array, table, and string replacements keep their TOML types for exact full-value placeholders; embedded placeholders always stringify the selected value. Prefer the inline `replacements = { ... }` form when the config also has top-level keys after it; a TOML `[replacements]` header keeps following keys in that table until the next table header.

## Run Configs

Run configs are TOML and are deep-merged over the built-in default run config template from `src/config_defaults/run.toml`.

Minimal shape:

```toml
name = "example"
target = { kind = "scalar", value = 1.23 } # optional
# or: target = { kind = "vector", components = { real = 1.23, imag = 0.0 } }

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
```

Optional capability requirements are matched against worker `capabilities` with `>=`:

```toml
evaluator_requirements = { gpu = 1, cuda = 12 }
sampler_requirements = { gpu = 1, madnis = 1 }
```

If `task_queue` is omitted, the run is created idle. Every run stores an initial root stage snapshot (`sequence_nr = 0`) immediately at creation.

### GammaLoop Evaluator

GammaLoop support is behind the default `gammaloop` Cargo feature. Build without the heavy GammaLoop dependency with:

```bash
cargo build --no-default-features
```

In that build, `evaluator.kind = "gammaloop"` and HwU histogram export return explicit unsupported-feature errors.

For `evaluator.kind = "gammaloop"`, `continuous_dims` and `discrete_dims` are inferred from the selected integrand and should be omitted.

`graph_groups = [0, 3]` optionally restricts a discretely sampled GammaLoop
integrand to those graph-group indices. The exposed domain renumbers the chosen
groups locally while evaluation maps them back to GammaLoop's original indices.
This lets separate child runs load and sample individual graph groups without
changing the generated state.

Gammaboard evaluates GammaLoop runs in x-space so GammaLoop's parameterized observable and histogram path is used.

`[evaluator.preprocessing]` is optional and runs GammaLoop commands after loading
the state and before integrand selection. `read_only = true` is the default and
documents the intended mode; once GammaLoop exposes read-only state loading,
GammaBoard will pass this flag to the state loader. Commands are executed in
order and may be any GammaLoop command.

### Process Evaluator

For `evaluator.kind = "process_evaluator"`, configure:

- `command`: exact process argv after `$resources` expansion, for example `["python", "-m", "my_runtime.evaluator_worker"]` or `["apptainer", "exec", "--nv", "--bind", "$resources:$resources", "$resources/runtimes/my_runtime/runtime.sif", "python", "-m", "my_runtime.evaluator_worker"]`.
- `cwd`: optional working directory; defaults to `$resources`.
- `domain`: explicit `Domain` tree. This is the authoritative coordinate layout for homogeneous and inhomogeneous runs.
- `components`: observable component names; defaults to `["value"]` and must match the vector accumulator components.
- `args = { ... }`: optional opaque JSON object passed to the process during `initialize`.

Domain config uses snake_case variants. Use `rectangular` for fixed-cardinality grids instead of expanding large branch trees:

```toml
domain = { rectangular = { discrete_cardinalities = [5, 5, 5], continuous_dims = 10 } }
```

It can be nested inside a discrete branch:

```toml
domain = { discrete = { axis_label = "channel", branches = [
  { index = 0, domain = { continuous = { dims = 3 } } },
  { index = 1, domain = { rectangular = { discrete_cardinalities = [5], continuous_dims = 5 } } },
] } }
```

Process evaluator construction semantics:

- GammaBoard only speaks the process protocol.
- The bundled Python package exposes `run_evaluator(MyEvaluator)`, where the worker module imports the user class explicitly.
- The bundled homogeneous Python wrapper derives `discrete_cardinalities` and `continuous_dims` from `domain` and calls `ClassName(discrete_cardinalities=..., continuous_dims=..., **args)`.
- Process evaluators require a `kind = "vector"` accumulator; the training projection is stored as its own scalar aggregate and is used for sampler feedback.

See [process-runtime.md](process-runtime.md) for the protocol contract.

### Process Sampler

For `sampler_aggregator.kind = "process_sampler"`, configure:

- `command`: exact process argv after `$resources` expansion, for example `["python", "-m", "my_runtime.sampler_worker"]` or `["apptainer", "exec", "--nv", "--bind", "$resources:$resources", "$resources/runtimes/my_runtime/runtime.sif", "python", "-m", "my_runtime.sampler_worker"]`.
- `cwd`: optional working directory; defaults to `$resources`.
- `requires_training_values`: set to `true` when the sampler needs evaluator feedback through `ingest_training_values`.
- `args = { ... }`: optional opaque JSON object passed to the process during `initialize`.

Process evaluator and sampler methods use ragged row-major arrays plus offsets. Homogeneous wrappers may validate that offsets match the fixed-width shape derived from `domain` and reject inhomogeneous batches.

Process samplers receive the authoritative run `domain`; sampler configs do not define coordinate shape separately.

Process sampler construction semantics:

- GammaBoard only speaks the process protocol.
- The bundled Python package exposes `run_sampler(MySampler)`, where the worker module imports the user class explicitly.
- Restore path: `from_snapshot(*, snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present.
- Fresh path: `ClassName(discrete_cardinalities=..., continuous_dims=..., **args)`.
- Python worker protocol entrypoints are included in each example runtime, but `command` must explicitly start the desired process.
- GammaBoard does not infer paths, append worker scripts, or inject Apptainer binds. Use `$resources` explicitly where the host resources path is needed.

## Task Queue

Sample tasks use direct source specs:

- Omit `evaluator` or use `evaluator = "latest"` to use the latest effective evaluator.
- Use `evaluator = { from_name = "..." }` to load the evaluator from a prior task snapshot.
- Use `evaluator = { config = ... }` to set an explicit task-local evaluator.
- Omit `sampler_aggregator` or `accumulator` to use `latest`.
- Use `kind = "set_accumulator"` when you want to establish or reset accumulator state explicitly before later sample tasks.
- Use `{ from_name = "..." }` to load from a prior task name.
- Use `{ config = ... }` to set explicit inline config.
- `accumulator = { config = "gammaloop" }` is available for GammaLoop runs and persists GammaLoop's native histogram snapshot bundle.

Task names are unique per run and can be referenced by `from_name`.

The top-level `[evaluator]` is shorthand for the initial evaluator stage. If it is omitted, the first explicit compute-task evaluator establishes the immutable run domain. Every later evaluator stage must resolve to that domain. Controller tasks (`parameter_scan`, `hyperparameter_tuning`, and `integration_campaign`) never resolve or inherit the parent evaluator; their child-run TOML owns each child evaluator. This keeps batches, materializers, and accumulator state compatible while allowing implementation and parameters to vary between compute stages.

`batch_transforms` is stage state for tasks. Omitted inherits; `batch_transforms = []` explicitly clears inherited transforms.

When raster `image`/`plot_line`/`pdf_adaptation_image`/`pdf_adaptation_plot_line` tasks should evaluate directly in declared geometry coordinates after transformed sampling stages, set `batch_transforms = []` on those raster tasks.

Raster geometry `discrete` selects the domain branch to scan. The selected branch, or remaining subtree if the path is a prefix, must determine a unique continuous dimensionality matching `offset` and direction vectors.

`set_accumulator` is the explicit no-work task for changing accumulator state. Sample tasks may omit `accumulator`, but only if a prior task in the run already established an effective accumulator state.

Task files used with `gammaboard run task append` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Sample task config example:

```toml
[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "scalar"

[task_queue.accumulator.discrete_projections]
normalization = "contribution" # default; use "conditional_mean" for per-bin means

[[task_queue.accumulator.discrete_projections.items]]
name = "summed"
dims = [0]
fixed_dims = {}

[[task_queue]]
name = "warmup-sample"
kind = "sample"
stop_condition = { max_samples = 10000, absolute_error = 1e-3, relative_error = 1e-2, projection = "real" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
```

Discrete projection dimensions address the discrete coordinate path. For inhomogeneous domains, samples where a configured fixed or projected dimension does not exist are ignored, so `fixed_dims` can select a subtree before projecting a deeper branch. The backend renders these projected scalar accumulators as histograms.

Deterministic scan tasks:

```toml
[[task_queue]]
kind = "image"
accumulator = "scalar"
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
u_linspace = { start = -2.0, stop = 2.0, count = 128 }
v_linspace = { start = -2.0, stop = 2.0, count = 128 }

[[task_queue]]
kind = "plot_line"
accumulator = "scalar"
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }

[[task_queue]]
kind = "pdf_adaptation_plot_line"
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }
```

## Integration Campaigns

`integration_campaign` is a control-plane task that owns independent child runs.
Each `[[task_queue.children]]` entry supplies a stable name, a finite coefficient,
and a complete child `run_toml`. The common `measurement` selects the child task
whose live or completed measurement is combined. Child estimates are assumed
independent, so uncertainties are combined in quadrature after applying the
coefficients. The controller also materializes live result snapshots from the
same child accumulator revisions. Compatible histogram bins are combined as
weighted values with independent variances in quadrature; incompatible or
missing layouts are listed as omitted. These larger payloads are stored as task
result snapshots rather than repeated in controller queue state.

Children may use entirely different evaluators, GammaLoop processes, or state
folders. Histogram summation is evaluator-independent and accepts arbitrary
GammaLoop continuous or discrete observables when every child publishes the
same observable name and bin layout. Each native child bundle remains available
through its child run even when an observable cannot be summed.

### Variance-based campaign allocation

For a campaign estimate

`I = sum_i c_i I_i`,

the controller assumes statistically independent child estimates and computes
the current weighted variance of child `i` as

`V_i = c_i^2 sum_k sigma_(i,k)^2`,

where `c_i` is the child's coefficient and `sigma_(i,k)` is the uncertainty of
measurement component `k`. Summing components gives the allocator one scalar
priority for vector or complex measurements. The combined result retains each
component separately and combines its child uncertainties in quadrature.

The default `allocation.algorithm = "variance_reduction_rate"` assigns the next
sample window by descending

`score_i = V_i r_i / N_i`,

where `N_i` is the child's sample count and `r_i` is its measured completed-sample
throughput. Monte Carlo variance approximately follows `V_i(N) = C_i / N`, so
its expected decrease per additional sample is proportional to `V_i / N_i`.
Multiplying by throughput estimates the decrease in total campaign variance per
wall-clock second. This directs work to the child expected to improve the final
precision fastest; it deliberately does not try to equalize child sample counts.

`allocation.algorithm = "largest_variance"` instead uses `score_i = V_i`. It is
useful when child evaluation costs are comparable or throughput should not affect
allocation. Unlike the default proxy, it does not explicitly divide by the
current sample count when estimating the marginal benefit of more samples.

Before scoring, children below `min_samples_per_child` receive pilot coverage,
least-sampled first. This supplies an initial uncertainty and throughput estimate
for every child. The selected set is retained until
`allocation_window_samples` additional child samples have completed, then scores
are recomputed; this avoids rapid reassignment from noisy live estimates.
`max_active_runs` selects how many of the highest-ranked children run concurrently.

The dashboard's **variance contribution (%)** is
`100 V_i / sum_j V_j`. It explains the current uncertainty budget, but is not the
default allocation score: throughput, sample count, the pilot phase, and the
current allocation window can make the selected child differ from the largest
displayed contributor. Percentages are withheld until every child has reported
the uncertainties needed for the total.

These calculations require independent child estimates. Correlated child
estimators need covariance terms and should not be represented as an independent
campaign. Early estimates can also be noisy; increase `min_samples_per_child` or
`allocation_window_samples` when allocations are unstable.

The campaign stops on its combined absolute or relative error after
`min_total_samples`, or at `max_total_samples`.

All controller child records include a result source reference containing the
child run, source task, immutable stage snapshot when available, and live sample
revision. Parameter scans use these records for their 1D series, 2D heatmaps,
and point table. Hyperparameter tuning exposes objective and best-so-far
history, plus `best_result_source` for the complete winning child result.

See `resources/templates/runs/integration-campaign-qft-like.toml` for a complete
two-graph-group ttH example with native GammaLoop histograms.

## Queue Tuning

Evaluators use a fixed single-slot latent prefetch and single-slot async submit pipeline. Materialization and evaluation still remain strictly one batch at a time.

`sampler_aggregator_runner_params` controls queue and persistence behavior:

- `frontend_sync_interval_ms` sets how often the sampler runner refreshes frontend-facing and persisted accumulator snapshots during sampling.
- Sampler queue settings live under `[sampler_aggregator_runner_params.queue]`.
- `target_batch_eval_ms`, `batch_size_deadband_ratio`, `batch_size_cooldown_ticks`, and `max_batch_size` are queue-level controls.
- `queue_buffer` targets about `queue_buffer * active_evaluator_count` pending batches. `0.0` is most aggressive; larger values keep more pending work buffered.
- Refill hysteresis is controlled with `pending_refill_low_ratio` and `pending_refill_high_ratio`.
- Total open batches (`pending + claimed + completed`) are capped by `max_queue_size`.
- Pause/unassign drains the local queue fully before the sampler checkpoint is persisted.
