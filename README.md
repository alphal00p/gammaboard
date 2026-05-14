# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Quickstart
Local development from the repo root:

```bash
just deploy local dev
```

This builds the dashboard when needed, builds the dev-optimized backend, starts local Postgres from [ops/local/config/runtime.toml](ops/local/config/runtime.toml), starts the backend, and serves the dashboard through nginx at `http://localhost:8080`. Stop the foreground deploy with `Ctrl-C`.

ITPhlies release deploy from the repo root on ITPhlies:

```bash
just deploy itphlies release
```

This builds the release backend and uses [ops/itphlies/config/runtime.toml](ops/itphlies/config/runtime.toml) plus [ops/itphlies/config/deploy.toml](ops/itphlies/config/deploy.toml). Open `http://itphlies:8080` on the LAN, or tunnel `ssh -N -L 8080:127.0.0.1:8080 ITPhliesTails` and open `http://localhost:8080`.

Run an additional isolated instance with a port offset:

```bash
just deploy local dev 1
just deploy itphlies release 1
```

`--port-offset 1` shifts frontend/API/Postgres from `8080/4000/5400` to `8081/4001/5401` and suffixes local Postgres state paths.

For UBELIX Slurm/Apptainer operation, use [ops/ubelix/README.md](ops/ubelix/README.md).

## Main Commands
- `gammaboard deploy run`: supervise local Postgres, backend API, and nginx/frontend in one foreground process.
- `gammaboard run`: create, list, pause, clone, remove, and append tasks to runs.
- `gammaboard node`: run workers, list nodes, assign roles, unassign, and request shutdown.
- `gammaboard db`: manage the local PostgreSQL instance used by the active runtime config.
- `gammaboard server`: run only the backend API. Use this for API-only/manual setups, not normal dashboard deploys.

The dashboard shows runs, task output, nodes, performance, and logs.

## Prerequisites
- Rust
- PostgreSQL 16 tools (`initdb`, `pg_ctl`, `postgres`, `psql`)
- `sqlx` CLI: `cargo install sqlx-cli --no-default-features --features postgres`
- Node.js + npm for building the dashboard frontend
- nginx for `gammaboard deploy run`
- `just` for the checked-in wrapper commands

## Deploy Without Wrappers
Use the deploy helper directly when you do not want the `just` wrappers but still want the normal dashboard stack.

Local dev profile:

```bash
just build-frontend
cargo build --profile dev-optim
./target/dev-optim/gammaboard deploy run --deploy-config ops/local/config/deploy.toml
```

ITPhlies release profile:

```bash
just build-frontend
cargo build --release
./target/release/gammaboard \
  --runtime-config ops/itphlies/config/runtime.toml \
  deploy run \
  --deploy-config ops/itphlies/config/deploy.toml
```

Useful deploy options:
- `--port-offset <N>` adds `N` to configured frontend/API/Postgres ports and suffixes local Postgres state paths for isolated multi-instance launches.
- `--api-port <PORT>` overrides the private backend API port for one launch.
- Global runtime overrides such as `--database-url`, `--postgres-data-dir`, `--postgres-socket-dir`, and `--postgres-log-file` can isolate instances without editing TOML files.

`gammaboard deploy run` validates the frontend build, optionally starts local Postgres, starts `gammaboard server` as a supervised child, generates nginx config, runs nginx in the foreground, and on `Ctrl-C`/`SIGTERM` requests graceful node shutdown before stopping nginx, backend, and local Postgres.

## Fully Manual API-Only Mode
Use this only if you do not want or need the dashboard frontend. `gammaboard server` starts the backend API directly and does not supervise nginx, build assets, or own a full deploy shutdown sequence.

Start and stop the local database:

```bash
gammaboard db status
gammaboard db start
gammaboard db stop
gammaboard db delete --yes
```

Start the API server:

```bash
gammaboard server --server-config ops/local/config/server.toml
```

Create a run, append tasks, and inspect state:

```bash
gammaboard run add resources/templates/runs/gammaloop.toml
gammaboard run task add gammaloop_tth resources/templates/tasks/train_sample.toml
gammaboard run list
gammaboard run task list gammaloop_tth
```

Start local workers and assign them:

```bash
gammaboard node auto-run 2
gammaboard node assign w-1 sampler-aggregator gammaloop_tth
gammaboard node assign w-2 evaluator gammaloop_tth
gammaboard node list
```

Pause work and shut down workers:

```bash
gammaboard run pause gammaloop_tth
gammaboard node stop -a
```

## Config Layout
- `src/config_defaults/*.toml`: built-in fallback defaults embedded into the Rust binary.
- [ops/local/config](ops/local/config): local development profiles.
- [ops/itphlies/config](ops/itphlies/config): ITPhlies profiles.
- `ops/ubelix/config/*`: UBELIX profiles rendered/submitted by the UBELIX launcher.
- `resources/templates/{runs,tasks,nodes}/`: shared templates served by the backend.

All commands load runtime config from [ops/local/config/runtime.toml](ops/local/config/runtime.toml) by default. Override it with:

```bash
gammaboard --runtime-config path/to/runtime.toml <COMMAND>
```

Deploy config points at the server config and frontend build path:

```toml
[api_server]
api_server_config = "server.toml"

[static_site]
frontend_build_dir = "../../../dashboard/build"

[frontend_http]
frontend_host = "127.0.0.1"
frontend_port = 8080
frontend_advertise_hosts = ["localhost"]

[database]
ensure_started = true
```

Runtime config owns the database URL, resource roots, tracing, and local Postgres settings. The checked-in default Postgres port is `5400`; `deploy run --port-offset <N>` shifts it at launch time.

Relative evaluator resources, such as GammaLoop `state_folder`, resolve through `resources.roots` in order. Absolute paths are used as-is.

## Frontend Routing
- The dashboard frontend always calls relative `/api` endpoints and does not require `.env`.
- Local Vite dev server still proxies `/api/*` to `http://127.0.0.1:4000` for frontend-only development.
- Production/dashboard deploys should serve frontend and backend behind the same origin. `gammaboard deploy run` does this with generated nginx config.

## Dashboard Auth
- Read-only dashboard endpoints stay open.
- Steering actions currently require admin login and are backed by a signed session cookie.
- The Runs tab includes a `Copy Run TOML` action that exports a run-add compatible TOML containing the run config and all successfully completed tasks as `[[task_queue]]` entries.
- The dashboard currently supports creating runs from raw TOML, cloning runs from a stored stage snapshot, appending tasks from raw TOML, deleting pending tasks, pausing runs, removing runs, auto-assigning free nodes, assigning and unassigning nodes, requesting node shutdown (single or all), creating grouped node launch requests, resetting the local database when `allow_db_admin = true`, and shutting down the control process.
- When `allow_local_node_spawn = true`, the server resolves node launch requests by spawning local child processes. Otherwise launch requests remain queued for an external launcher. External launchers should use the node-launch-request API; `starting` means workers were submitted, and `running` is reconciled from live node leases.
- The Performance tab defaults history x-axes to sampler uptime (active sampler runtime, excluding paused intervals) and lets operators switch to wall time or total completed samples.
- The create-run, add-task, and node-request dialogs can load `.toml` templates from `run_templates_dir`, `task_templates_dir`, and `node_templates_dir`; admin users can also save edited TOML back as templates and delete templates from the dashboard.
- Node shutdown from the dashboard is guarded by a confirmation dialog.
- Put `auth.admin_password_hash` in your server config to enable dashboard auth.
- Put `auth.session_secret` in your server config when auth is enabled.
- Set `allowed_origins` in your server config if the frontend is served from origins other than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in your server config.
- Generate the password hash with:
  ```bash
  gammaboard auth --password 'your-password'
  ```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

## Run Configs
Run configs are TOML and are deep-merged over the built-in default run config template from `src/config_defaults/run.toml`.

GammaLoop support is behind the default `gammaloop` Cargo feature. Build without the heavy GammaLoop dependency with:

```bash
cargo build --no-default-features
```

In that build, `evaluator.kind = "gammaloop"` and HwU histogram export return a clear unsupported-feature error.

Add a run with:
```bash
gammaboard run add resources/templates/runs/gammaloop.toml
```

Flake-backed process evaluator + sampler example:
```bash
gammaboard run add resources/templates/runs/process-evaluator-process-sampler-flake-demo.toml
```

Apptainer-backed Rust process evaluator example:
```bash
cd process_api/examples/rust_breit_wigner_evaluator
apptainer build runtime.sif apptainer.def
cd ../../..
gammaboard run add resources/templates/runs/process-rust-apptainer-evaluator-demo.toml
```

Curated run templates:
- `resources/templates/runs/ghost_bump.toml`: Symbolica + Havana training on a 2D `(x, y)` domain plus `pdf_adaptation_image` for ghost-bump diagnostics.
- `resources/templates/runs/symbolica-havana-pdf-1d2d.toml`: Symbolica + Havana training + both PDF adaptation task kinds (`pdf_adaptation_image`, `pdf_adaptation_plot_line`).
- `resources/templates/runs/process-evaluator-process-sampler-flake-demo.toml`: Process evaluator and sampler integration.
- `resources/templates/runs/process-rust-apptainer-evaluator-demo.toml`: Apptainer-packaged Rust process evaluator integration.
- `resources/templates/runs/gammaloop.toml`: GammaLoop TTH evaluator config, including optional `post_load_commands`.

Curated task bundles:
- `resources/templates/tasks/sample_monte_carlo_real.toml`: minimal scalar sample task with naive Monte Carlo.
- `resources/templates/tasks/pdf_adaptation_image.toml`: Havana training followed by PDF adaptation image rasterization.
- `resources/templates/tasks/train_sample.toml`: GammaLoop TTH train+sample queue with queue tuning and inference stop target (`relative_error = 0.001`, `max_samples = 1_000_000_000`).

Curated node launch templates:
- `resources/templates/nodes/local-two-workers.toml`: two local workers for local/ITPhlies development. UBELIX-specific node templates live under `ops/ubelix/config/templates/nodes`.

Minimal shape:
```toml
name = "example"
target = { kind = "scalar", value = 1.23 } # optional
# or: target = { kind = "complex", re = 1.23, im = 0.0 }

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
```

Optional capability requirements (matched against worker `capabilities` with `>=`):

```toml
evaluator_requirements = { gpu = 1, cuda = 12 }
sampler_requirements = { gpu = 1, madnis = 1 }
```

For `evaluator.kind = "gammaloop"`, `continuous_dims` and `discrete_dims` are inferred from the selected integrand and should be omitted.
Gammaboard evaluates GammaLoop runs in x-space so GammaLoop's parameterized observable and histogram path is used.
`post_load_commands = ["set ...", ...]` is optional and executes in-memory after loading the GammaLoop state and before integrand selection; commands are not saved back to disk.

For `evaluator.kind = "process_evaluator"`, configure:
- `command`: complete process command, for example `["nix", "shell", "path:./process_api/examples/python_scalar_sin#runtime", "-c", "gammaboard-example-evaluator-worker"]` or `["apptainer", "exec", "--nv", "runtimes/my_runtime/runtime.sif", "python", "-u", "runtimes/my_runtime/evaluator_worker.py"]`
- `continuous_dims`: expected continuous dimension for homogeneous rectangular batches
- `discrete_cardinalities`: expected per-axis discrete cardinalities for homogeneous rectangular batches (for example `[3, 4, 2]`)
- `components`: observable component names; currently exactly one component is supported by the scalar accumulator path and defaults to `["value"]`
- optional `args = { ... }`: opaque JSON object passed to the process during initialize

Process evaluator construction semantics:
- GammaBoard only speaks the process protocol; the bundled Python wrapper uses `args.module` and `args.class` to import a class exposing `eval(xs_discrete, xs_continuous)`.
- if the Python class defines `from_config(discrete_cardinalities=..., continuous_dims=..., init_args=...)`, that is called with `args` excluding `module` and `class`
- otherwise the worker calls `ClassName(**args)` (or `ClassName()` when `args` is empty)

For `sampler_aggregator.kind = "process_sampler"`, configure:
- `command`: complete process command, for example `["nix", "shell", "path:./process_api/examples/python_sampler_symbolica_havana#runtime", "-c", "gammaboard-example-sampler-worker"]` or `["apptainer", "exec", "--nv", "runtimes/my_runtime/runtime.sif", "python", "-u", "runtimes/my_runtime/sampler_worker.py"]`
- `continuous_dims`: expected homogeneous continuous dimension
- `requires_training_values`: set to `true` when the sampler needs evaluator feedback through `ingest_training_values`
- optional `args = { ... }`: opaque JSON object passed to the process during initialize
- Process evaluator and sampler methods are vectorized over fixed rectangular batches:
  `xs_discrete` has shape `(nr_samples, len(discrete_cardinalities))` and `xs_continuous` has shape `(nr_samples, continuous_dims)`.
- Process samplers receive `discrete_cardinalities` derived from the run domain, not a separate sampler config field.
- `produce_latent_batch(nr_samples)` returns an object with `xs_discrete`, `xs_continuous`, and `weights` attributes when using the Python wrapper.
- `weights` are the per-sample multipliers stored as the `sampler_weight` factor before accumulation.
- optional `pdf(xs_discrete, xs_continuous)` returns `(nr_samples,)`.
- the checked-in Symbolica Havana example lives at `process_api/examples/python_sampler_symbolica_havana` and packages its Symbolica Python wheel in the example flake

Process sampler construction semantics:
- GammaBoard only speaks the process protocol; the bundled Python wrapper uses `args.module` and `args.class` to import a class implementing sampler methods (`sample_plan`, `training_samples_remaining`, `produce_latent_batch`, `ingest_training_values`, `snapshot`, optional `pdf`).
- restore path: `from_snapshot(snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present, with `args` excluding `module` and `class`
- fresh path: `from_config(discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present, with `args` excluding `module` and `class`
- fallback: `ClassName(**args)` / `ClassName()`
- Python worker protocol entrypoints are included in each example runtime, but `command` must explicitly start the desired process.
- Relative command entries that look like paths, such as `runtimes/my_runtime/runtime.sif`, resolve under the resources root.
- Worker processes speak `gammaboard-jsonrpc-v1`: Content-Length framed JSON-RPC over stdin/stdout, with stderr reserved for logs. The Python wrappers redirect ordinary `print()` output to stderr and keep a private stdout handle for protocol frames.
- See `process_api/README.md` for the concise process protocol and Python wrapper guide.

If `task_queue` is omitted, the run is created idle.
Every run stores an initial root stage snapshot (`sequence_nr = 0`) immediately at creation.

### Task Queue
Sample tasks use direct per-component source specs:
- omit `sampler_aggregator` or `accumulator` to use `latest`
- use `kind = "set_accumulator"` when you want to establish or reset accumulator state explicitly before later sample tasks
- use `{ from_name = "..." }` to load from a prior task name
- use `{ config = ... }` to set explicit inline config
- `accumulator = { config = "gammaloop" }` is available for GammaLoop runs and persists GammaLoop's native histogram snapshot bundle

Task names are unique per run and can be referenced by `from_name`.
`batch_transforms` is stage state for tasks. Omitted inherits; `batch_transforms = []` explicitly clears inherited transforms.
When you want raster `image`/`plot_line`/`pdf_adaptation_image`/`pdf_adaptation_plot_line` tasks to evaluate directly in declared geometry coordinates after transformed sampling stages, set `batch_transforms = []` on those raster tasks.
`set_accumulator` is the explicit no-work task for changing accumulator state. Sample tasks may omit `accumulator`, but only if a prior task in the run already established an effective accumulator state.
Task files used with `gammaboard run task add` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Sample task config example:
```toml
[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "scalar"

[task_queue.accumulator.discrete_histograms]
max_total_bins = 4096
normalization = "contribution" # default; use "conditional_mean" for per-bin means

[[task_queue.accumulator.discrete_histograms.items]]
name = "summed"
hist_dims = [0]
fixed_dims = {}

[[task_queue.accumulator.discrete_histograms.items]]
name = "channel_1"
hist_dims = [1, 2]
fixed_dims = { "0" = 0 }

[[task_queue]]
name = "warmup-sample" # optional; auto-generated when omitted
kind = "sample"
stop_condition = { max_samples = 10000, absolute_error = 1e-3, relative_error = 1e-2, projection = "real" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
```

Evaluators use a fixed single-slot latent prefetch and single-slot async submit pipeline. Materialization and evaluation still remain strictly one batch at a time.

`sampler_aggregator_runner_params` also controls queue and persistence behavior:
- `frontend_sync_interval_ms` sets how often the sampler runner refreshes the frontend-facing accumulator state and persisted accumulator snapshots during sampling; the shared run default is `2000`.
- Sampler queue settings live under the nested TOML table `[sampler_aggregator_runner_params.queue]`.
- `target_batch_eval_ms`, `batch_size_deadband_ratio`, `batch_size_cooldown_ticks`, and `max_batch_size` are queue-level controls under `[sampler_aggregator_runner_params.queue]`; the queue owns batch-size tuning during production planning.
- `queue_buffer` is the single queue buffer knob for the sampler queue. The runner targets about `queue_buffer * active_evaluator_count` pending batches. A value of `0.0` is the most aggressive and lets the queue drain to zero pending batches when the sampler cannot refill it faster than evaluators consume work. Larger values keep more pending work buffered. `max_queue_size` remains the hard cap.
- Refill hysteresis is controlled with `pending_refill_low_ratio` and `pending_refill_high_ratio`. Production starts when pending drops to the low watermark and refills toward the high watermark.
- Total open batches (`pending + claimed + completed`) are still capped by `max_queue_size`.
- The sampler queue owns a local pending-insert buffer and a local processed buffer. Polling completed work is non-blocking during normal ticks, and pause/unassign drains the local queue fully before the sampler checkpoint is persisted.

Deterministic scan tasks are supported:
```toml
[[task_queue]]
kind = "image"
accumulator = "complex"
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
u_linspace = { start = -2.0, stop = 2.0, count = 128 }
v_linspace = { start = -2.0, stop = 2.0, count = 128 }

[[task_queue]]
kind = "plot_line"
accumulator = "complex"
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
# sampler_aggregator omitted => latest; or use { from_name = "..." }
```

## Runs And Names
- Run names are not unique.
- CLI run arguments accept either a numeric id or an exact name.
- If a name matches multiple runs, the CLI prints the matches and asks for an id.

List runs:
```bash
gammaboard run list
gammaboard run list my-run-name
```

Clone a run branch from a specific stage snapshot:
```bash
gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
```
Clone creates a new run rooted at that snapshot and does not copy queued tasks from the source run.
In the dashboard, clone source is inferred from the selected task (falling back to the run root snapshot).

## Nodes
Start local workers:
```bash
gammaboard node auto-run 2
```

Or directly:
```bash
gammaboard node run --name w-1
```

`node run` uses a fast-start reconcile backoff internally: it starts polling at `50ms`, grows by a factor of `2.0`, and caps at `2s`.
`node run` starts graceful shutdown on `Ctrl-C` and `SIGTERM`, and expires its lease on shutdown so the same node name can be reused immediately.
`node auto-run N` picks names `w-1`, `w-2`, ... and skips names that already exist in the control plane.
`node auto-run` uses a moderate default `--db-pool-size 4` to reduce worker-side pool contention while still keeping large fanout manageable under database connection pressure.
Auto-run workers now write per-node startup logs to `logs/nodes/<NODE_NAME>.stdout.log` and `logs/nodes/<NODE_NAME>.stderr.log`.
If an auto-run child exits unsuccessfully, the parent control process logs the exit status together with those log paths.

Node names are unique operator handles. Each live worker also owns an internal UUID lease in PostgreSQL. If the worker cannot re-announce itself for 30 seconds, it shuts down.
Removing a run is immediate: Gammaboard clears node assignments for that run and deletes the run data without waiting for pause/drain persistence.
Evaluator-side batch errors are retried as batch-local failures and do not fail the run; sampler-aggregator failures can still fail the active task because sampler state owns task progress.

Assign roles:
```bash
gammaboard node assign w-1 evaluator <RUN>
gammaboard node assign w-2 sampler-aggregator <RUN>
```

Auto-assign currently free nodes:
```bash
gammaboard auto-assign <RUN> [MAX_EVALUATORS]
```

## Common Commands
```bash
gammaboard run list [RUN_NAME]
gammaboard run pause <RUN>
gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
gammaboard run task list <RUN>
gammaboard run task add <RUN> <TASK_FILE.toml>
gammaboard run task remove <RUN> <TASK_ID>
gammaboard run remove <RUN>

gammaboard node list
gammaboard node run --name <NODE_NAME>
gammaboard node auto-run <COUNT>
gammaboard node assign <NODE_NAME> <ROLE> <RUN>
gammaboard node unassign <NODE_NAME>
gammaboard node stop <NODE_NAME>
```

## Useful Local Commands
```bash
gammaboard run pause -a
gammaboard node stop -a
cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1
```
