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
connect to the local Postgres instance.

Relative evaluator resources, such as GammaLoop `state_folder`, resolve through `resources.roots` in order. Absolute paths are used as-is.

## Server Config

Server config owns both the backend API settings and the foreground deploy
settings used by `gammaboard deploy run`:

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

`gammaboard deploy run` validates the frontend build, optionally starts local Postgres, starts `gammaboard server` as a supervised child, generates nginx config, runs nginx in the foreground, and on `Ctrl-C`/`SIGTERM` requests graceful node shutdown before stopping nginx, backend, and local Postgres.

## Runtime Config

Runtime config owns the database URL, resource roots, tracing, and local Postgres settings. The checked-in default Postgres port is `5400`; global `--port-offset <N>` shifts frontend/API/Postgres ports and local Postgres state paths for all child processes.

## Auth And Templates

- Read-only dashboard endpoints stay open.
- Steering actions require admin login only when `[auth]` is configured.
- Omit `[auth]` for passwordless local control.
- Put `auth.admin_password_hash` and `auth.session_secret` in your server config to enable dashboard auth.
- Set `allowed_origins` in your server config if the frontend is served from origins other than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in your server config.

Generate the password hash with:

```bash
gammaboard auth --password 'your-password'
```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

When `allow_local_node_spawn = true`, the server resolves node launch requests by spawning local child processes. Otherwise launch requests remain queued for an external launcher. External launchers should use the node-launch-request API; `starting` means workers were submitted, and `running` is reconciled from live node leases.

The create-run, add-task, and node-request dialogs can load `.toml` templates from `run_templates_dir`, `task_templates_dir`, and `node_templates_dir`; admin users can also save edited TOML back as templates and delete templates from the dashboard.

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

Gammaboard evaluates GammaLoop runs in x-space so GammaLoop's parameterized observable and histogram path is used.

`[evaluator.preprocessing]` is optional and runs GammaLoop commands after loading
the state and before integrand selection. `read_only = true` is the default and
documents the intended mode; once GammaLoop exposes read-only state loading,
GammaBoard will pass this flag to the state loader. Commands are executed in
order and may be any GammaLoop command.

### Process Evaluator

For `evaluator.kind = "process_evaluator"`, configure:

- `command`: exact process argv after `$resources` expansion, for example `["python", "-u", "$resources/runtimes/my_runtime/evaluator_worker.py"]` or `["apptainer", "exec", "--nv", "--bind", "$resources:$resources", "$resources/runtimes/my_runtime/runtime.sif", "python", "-u", "/opt/my_runtime/evaluator_worker.py"]`.
- `cwd`: optional working directory; defaults to `$resources`.
- `domain`: explicit `Domain` tree. This is the authoritative coordinate layout for homogeneous and inhomogeneous runs.
- `components`: observable component names; defaults to `["value"]` and must match the vector accumulator components.
- `args = { ... }`: optional opaque JSON object passed to the process during `initialize`; GammaBoard does not expand placeholders inside `args`.

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
- The bundled Python wrapper uses `args.module` and `args.class` to import a class exposing `eval(xs_discrete, xs_continuous)`.
- If the Python class defines `from_config(discrete_cardinalities=..., continuous_dims=..., init_args=...)`, the bundled homogeneous Python wrapper derives those values from `domain` and calls it with `args` excluding `module` and `class`.
- Otherwise the worker calls `ClassName(**args)` or `ClassName()` when `args` is empty.
- Process evaluators require a `kind = "vector"` accumulator; the training projection is stored as its own scalar aggregate and is used for sampler feedback.

See [process-runtime.md](process-runtime.md) for the protocol contract.

### Process Sampler

For `sampler_aggregator.kind = "process_sampler"`, configure:

- `command`: exact process argv after `$resources` expansion, for example `["python", "-u", "$resources/runtimes/my_runtime/sampler_worker.py"]` or `["apptainer", "exec", "--nv", "--bind", "$resources:$resources", "$resources/runtimes/my_runtime/runtime.sif", "python", "-u", "/opt/my_runtime/sampler_worker.py"]`.
- `cwd`: optional working directory; defaults to `$resources`.
- `requires_training_values`: set to `true` when the sampler needs evaluator feedback through `ingest_training_values`.
- `args = { ... }`: optional opaque JSON object passed to the process during `initialize`; GammaBoard does not expand placeholders inside `args`.

Process evaluator and sampler methods use ragged row-major arrays plus offsets. Homogeneous wrappers may validate that offsets match the fixed-width shape derived from `domain` and reject inhomogeneous batches.

Process samplers receive the authoritative run `domain`; sampler configs do not define coordinate shape separately.

Process sampler construction semantics:

- GammaBoard only speaks the process protocol.
- The bundled Python wrapper uses `args.module` and `args.class` to import a class implementing sampler methods.
- Restore path: `from_snapshot(snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present, with `args` excluding `module` and `class`.
- Fresh path: `from_config(discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present, with `args` excluding `module` and `class`.
- Fallback: `ClassName(**args)` / `ClassName()`.
- Python worker protocol entrypoints are included in each example runtime, but `command` must explicitly start the desired process.
- GammaBoard does not infer paths, append worker scripts, or inject Apptainer binds. Use `$resources` explicitly where the host resources path is needed.

## Task Queue

Sample tasks use direct source specs:

- Omit `sampler_aggregator` or `accumulator` to use `latest`.
- Use `kind = "set_accumulator"` when you want to establish or reset accumulator state explicitly before later sample tasks.
- Use `{ from_name = "..." }` to load from a prior task name.
- Use `{ config = ... }` to set explicit inline config.
- `accumulator = { config = "gammaloop" }` is available for GammaLoop runs and persists GammaLoop's native histogram snapshot bundle.

Task names are unique per run and can be referenced by `from_name`.

`batch_transforms` is stage state for tasks. Omitted inherits; `batch_transforms = []` explicitly clears inherited transforms.

When raster `image`/`plot_line`/`pdf_adaptation_image`/`pdf_adaptation_plot_line` tasks should evaluate directly in declared geometry coordinates after transformed sampling stages, set `batch_transforms = []` on those raster tasks.

Raster geometry `discrete` selects the domain branch to scan. The selected branch, or remaining subtree if the path is a prefix, must determine a unique continuous dimensionality matching `offset` and direction vectors.

`set_accumulator` is the explicit no-work task for changing accumulator state. Sample tasks may omit `accumulator`, but only if a prior task in the run already established an effective accumulator state.

Task files used with `gammaboard run task add` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Sample task config example:

```toml
[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "scalar"

[task_queue.accumulator.discrete_projections]
max_total_bins = 4096
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

## Templates

Curated run templates:

- `resources/templates/runs/ghost_bump.toml`: Symbolica + Havana training on a 2D `(x, y)` domain plus `pdf_adaptation_image`.
- `resources/templates/runs/symbolica-havana-pdf-1d2d.toml`: Symbolica + Havana training + both PDF adaptation task kinds.
- `resources/templates/runs/process-rust-apptainer-evaluator-demo.toml`: Apptainer-packaged Rust process evaluator integration.
- `resources/templates/runs/process-evaluator-process-sampler-demo.toml`: Process evaluator and sampler integration using Nix as one packaging option.
- `resources/templates/runs/gammaloop.toml`: GammaLoop TTH evaluator config.

Curated task bundles:

- `resources/templates/tasks/sample_monte_carlo_real.toml`: minimal scalar sample task with naive Monte Carlo.
- `resources/templates/tasks/pdf_adaptation_image.toml`: Havana training followed by PDF adaptation image rasterization.
- `resources/templates/tasks/train_sample.toml`: GammaLoop TTH train+sample queue with queue tuning and inference stop target.

Curated node launch templates:

- `resources/templates/nodes/local-two-workers.toml`: two local workers for local/ITPhlies development.
- UBELIX-specific node launch templates live under `ops/ubelix/resources/templates/nodes`.
