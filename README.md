# GammaBoard

GammaBoard runs distributed numerical integration jobs with PostgreSQL as the
shared control plane. The dashboard shows runs, task output, nodes, performance,
logs, and active runtime settings.

## Quickstart

The shortest supported setup uses Nix on Linux. From a fresh clone:

```bash
nix develop
./gammaboard deploy
```

This builds GammaBoard and the dashboard, starts the managed PostgreSQL
database, and opens the dashboard at `http://localhost:8080`. Leave it running.

In a second terminal, enter the same repository and run the dependency-free
installation smoke test:

```bash
nix develop
./gammaboard run create resources/templates/runs/installation-smoke.toml
./gammaboard node start-local 2
./gammaboard node auto-assign installation-smoke
```

Open the `installation-smoke` run in the dashboard. It integrates
`f(x) = 1` on `[0,1]`, normally finishes within a few seconds, and should report
a central value of `1.0`. Stop the workers and deployment with:

```bash
./gammaboard node stop -a
# Press Ctrl-C in the deployment terminal.
```

This local workflow intentionally uses passwordless PostgreSQL and dashboard
control on loopback. Security configuration is optional. A
deployment is an operator tool: anyone who can reach a passwordless deployment,
or knows its configured admin password, has full administrative access,
including creating runs that launch configured process commands. Keep that
access to trusted people and networks.

For setup without Nix, shared-machine use, and physics integrations, continue
with:

- [docs/quickstart.md](docs/quickstart.md)
- [docs/requirements.md](docs/requirements.md)
- [docs/README.md](docs/README.md)
- [integrations/README.md](integrations/README.md)

If you use Nix, the repository flake provides a development/operator shell with
Rust, Node.js, PostgreSQL, nginx, Apptainer, and helper tools. It is not the
deployment mechanism; production runs use the `gammaboard` binary, ops configs,
and, on UBELIX, Apptainer/Slurm images.

For ITPhlies:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  deploy \
  --server-config ops/itphlies/config/server.toml
```

For UBELIX Slurm/Apptainer operation, use
[ops/ubelix/README.md](ops/ubelix/README.md).

Validate an example before allocating workers:

```bash
./gammaboard run validate example.toml
./gammaboard run validate example.toml --probe
```

Validation shares creation's task/domain checks and checks process executables,
working directories and generated state artifacts on the current host. `--probe`
also initializes configured runtimes (including requested GPU initialization)
without creating a run or producing samples. Run it in the worker environment
when its executables or resource mounts differ from the control host.

## Core Commands

- `gammaboard deploy`: supervise local Postgres, backend API, and nginx/frontend in one foreground process.
- `gammaboard run`: create, list, pause, clone, remove, and append tasks to runs.
- `gammaboard node`: run workers, list nodes, assign roles, unassign, and request shutdown.
- `gammaboard db`: manage the local PostgreSQL instance used by the active runtime config.
- `gammaboard server`: run only the backend API for API-only/manual setups.

Run `gammaboard --version` to print the release version.

The repo-root `./gammaboard` helper builds the current CLI before forwarding
arguments to it. It uses `dev-optim` by default, `GAMMABOARD_PROFILE=release`
for release builds, and also builds the dashboard frontend for `deploy`.
The frontend build is skipped when its output is newer than its sources and
configuration. Pass `./gammaboard deploy --rebuild-frontend` to force a rebuild.
Set `GAMMABOARD_FRONTEND_BASE=/board/` when building the dashboard for a
reverse-proxy mount below a URL path instead of at `/`.

When opening the dashboard through a forwarded port or a different hostname,
allow the origin visible in your browser (scheme, host, and port, without a path):

```bash
./gammaboard deploy --allowed-origin http://localhost:39491
```

The option is repeatable and also available on `gammaboard server`. For a
persistent deployment, add the origin to `server.allowed_origins` in
`server.toml`. A changed external port needs an updated origin. CLI origins are
added after `--port-offset` is applied; configured local origins with explicit
ports are shifted by that offset. The dashboard checks browser access before
loading workspaces and shows a configuration remedy if the origin is rejected.

## Core Ideas

GammaBoard separates integration into a persisted control plane and replaceable
workers that can come and go.

- A run is the immutable top-level problem definition: domain, initial evaluator
  stage, default runner settings, and the task queue.
- A task is one step on the run timeline, such as sampling, plotting, setting an
  accumulator, parameter scanning, hyperparameter tuning, or steering an integration
  campaign across independent sub-runs.
- A snapshot records the stage state after a task: accumulator state, sampler
  state, evaluator config, and batch transform config. Later tasks restore from
  these snapshots instead of relying on in-memory handoff.
- A node is a live worker process registered in PostgreSQL by `name` plus
  process `uuid`. Nodes receive sampler-aggregator or evaluator assignments
  from the control plane.
- CPU-hours measure allocated sampler/evaluator worker core-time: assignment
  wall time multiplied by the node's declared `cpus` capability, or one CPU when
  omitted. Task counters persist across reassignment and completion; controller
  tasks and parent runs include all descendant child-run CPU-hours.
- The supervisor leader activates pending tasks, runs controller tasks, and
  updates node assignments. It does not consume evaluator/sampler compute slots.
- Sampler aggregators own sample production. They decide when to produce work,
  emit latent batches into the queue, ingest evaluator feedback when training is
  enabled, expose optional PDFs/diagnostics, and persist sampler snapshots.
- Materializers convert queued latent batches into concrete evaluator-side
  batches. Most samplers use identity materialization; specialized samplers can
  use built-in materializers or a `process_materializer`.
- Evaluators consume concrete batches, validate them against the run domain,
  evaluate the integrand, and return accumulator updates plus optional scalar
  training values for adaptive samplers.
- Task rate and ETA use completed samples over the last 60 seconds of active
  sampler wall time, including training and queue waits. Evaluator Busy measures
  materialization/evaluation time over a 60-second worker wall-time window,
  including polling sleeps; it is not operating-system CPU utilization. Windows
  start fresh on runner activation. Throughput history still reports completed
  batches per snapshot interval, so its chart can show spikes while workers are busy.
- Accumulators own observable semantics: scalar/vector/full-vector/GammaLoop
  state, error estimates, moments, projections, and panel-ready metrics.

The hot path is:

```text
sampler aggregator -> latent batch queue -> materializer -> batch transforms -> evaluator -> accumulator snapshot/training feedback
```

Queue defaults target 2 seconds of evaluation per batch and one pending
batch per active evaluator, counting queued and unpersisted work together.
Workers use a 10 ms minimum polling interval; longer evaluation calls need no
additional sleep. Task-level `queue_tuning` overrides apply live through the
dashboard. A single `queue_buffer` sets the pending target; the separate refill
low/high ratios and local buffer multiplier have been removed. Batch
sizing uses a 15% deadband and waits for three completed evaluation batches
between changes (`batch_size_cooldown_ticks` counts these observations, not
worker polling ticks). The maximum batch size and queue/I/O limits remain independent
safety bounds; increasing the pending buffer cannot make an adaptive sampler
produce past its training boundary.

Finite training windows aim for at least four chunks per evaluator, subject to
minimum batch size and queue limits. Fresh batches give evaluators without work
priority over speculative prefetch for up to 250 ms; older work remains claimable
if a peer is unresponsive. Evaluation-time smoothing uses an EWMA weight of 0.2
per 1,000 samples for both queue sizing and evaluator timing statistics, retaining
history across normal-sized batches. These defaults
are starting points: adapters with substantial per-batch setup can benefit from
longer batches, while finite training windows need enough chunks for parallelism.

Sampler timing panels report observations from each performance snapshot
interval. They do not maintain a second checkpointed smoothing history. Queue
control uses its own sample-weighted evaluation-time estimate, independently of
those diagnostic intervals. Queue insert/fetch/cleanup durations end when their
I/O finishes, excluding the wait until the sampler collects the result.

The run domain is authoritative throughout this path. Samplers produce points in
that domain, materializers and transforms must preserve a valid concrete batch,
and evaluator workers validate materialized/transformed batches before calling
the configured evaluator.

## Process Runtimes

External evaluators, samplers, transforms, and materializers speak framed
JSON-RPC over stdin/stdout. The protocol is in
[docs/process-runtime.md](docs/process-runtime.md); the Python helpers and
working runtimes are in [process_api](process_api). GammaBoard isolates these
workers from terminal signals and owns their EOF-first, bounded shutdown.

## Controller results

Controller tasks keep their children as normal inspectable runs and expose a
small result reference for each child. A reference identifies the source run,
task, immutable stage snapshot when available, and live sample revision.
Measurements are scalar selectors used for stopping and optimization; full
scientific results remain separate from runtime throughput and controller state.

- `parameter_scan` exposes completed point metrics as 1D series or a 2D heatmap,
  together with the full parameter table and child result references.
- `hyperparameter_tuning` exposes objective history, best-so-far history, all
  trial parameters, and a direct result reference for the best trial.
- `integration_campaign` continuously materializes a parent result from the
  latest child revisions. Metrics and compatible histogram bins are summed with
  their signed coefficients; independent variances are combined in quadrature.
  Incompatible histogram layouts are reported as omitted rather than pooled.

Campaign allocation targets uncertainty reduction, not equal sample counts.
After giving every child its configured pilot samples, the default policy ranks
each child by `weighted variance * throughput / samples`: the estimated decrease
in total campaign variance per wall-clock second. Allocation is reconsidered at
sample-window boundaries to avoid rapid worker churn. The table's variance
contribution is only `weighted variance / total variance`; it can therefore
differ from the allocation order when child throughputs or sample counts differ.
See [docs/config.md](docs/config.md#variance-based-campaign-allocation) for the
formula and assumptions.

Controller plots and child tables remain available after completion. Clicking
a scan point, tuning trial, or campaign entry opens that persistent child run.

Campaign result snapshots are persisted independently of the frequently polled
controller state. Consequently the dashboard can update combined histograms
while sampling is active without placing the histogram bundle in every task-list
response. The final snapshot records exactly which child revisions contributed.

Examples are available in `resources/templates/runs/parameter-scan-symbolica.toml`,
`resources/templates/runs/hyperparameter-tuning-symbolica.toml`, and the
QFT-like `resources/templates/runs/integration-campaign-qft-like.toml`.
The campaign example uses a generated GammaLoop ttH state and restricts its two
children to graph groups GL0 and GL2 via `evaluator.graph_groups`; their native
GammaLoop histogram bundles are combined live in the campaign view. Generate
the state with the same GammaLoop revision used to build GammaBoard, or override
the template's `state_folder` replacement.

## Development

```bash
cargo test -q
just test-e2e
```

Build without the heavy GammaLoop dependency:

```bash
cargo build --no-default-features
```

## License

GammaBoard is licensed under the MIT License.

Normal GammaBoard builds include OEM-licensed Symbolica activation, so users do
not need to obtain or configure a separate Symbolica license. Symbolica remains
subject to its own license terms: https://symbolica.io/license.html

Builds without the default `gammaloop` feature do not link GammaLoop, but
GammaBoard still depends directly on Symbolica for built-in Symbolica
evaluators. Developers who explicitly compile with
`NO_SYMBOLICA_OEM_LICENSE=1` must provide `SYMBOLICA_LICENSE` at runtime.

Graceful `deploy` shutdown automatically marks live workers for later recreation and
preserves their intended assignments. Use `gammaboard deploy --resume-workers` to
consume those markers through the normal launch-request queue. Plain `deploy`
leaves them dormant. Explicit `node stop` clears a worker's marker. Pausing or
unassigning a run still clears its assignments. Crashes and forced kills cannot
save a new shutdown roster or guarantee a checkpoint.

Launch workers through `node start-local`, the dashboard launch form, or the
UBELIX launcher so their complete launch configuration is recorded. Workers
started manually with `node run` have no reproducible launch request and are
reported as unavailable for automatic recreation. Pending external jobs are not
part of the saved roster; manage them through their existing launch requests.
UBELIX `down` saves the roster before stopping jobs; `up --resume-workers --watch`
re-enqueues saved workers with their original scheduler settings.

The dashboard startup queue shows pending/starting requests and failures. Fulfilled
and canceled requests appear in collapsed launch history (latest 100). A fulfilled
request means its workers connected, not that they are still online; current health
is shown in the node list. Resuming a worker reuses its name and creates a new
launch request, preserving the earlier attempts. Outstanding requests survive
redeployment and are never hidden by the history limit.

PostgreSQL store tests require `GAMMABOARD_TEST_DATABASE_URL` pointing to an
isolated, migrated test database. They fail if it is missing or unavailable;
they never fall back to a running deployment. Full-stack tests also accept this
variable and create their own temporary databases.

The run's **Checkpoint recovery** panel and `run inspect` show saving/saved/failed
status, the saved task and sample count, and the last restore time and worker.
A successful save is reported only after the checkpoint transaction commits.
Checkpoint decode errors fail activation; completed work without a resume
checkpoint is not silently restarted from fresh sampler state.

Worker details include the current activity, its start time, and the age of the
last completed batch. The lease heartbeat publishes activity independently of
blocked sampler/evaluator calls. Updates are capped at the heartbeat frequency.
Throughput is a 60-second active runner wall-time window, including waits.

Process workers may send framed JSON-RPC notifications before their response:
`{"jsonrpc":"2.0","method":"progress","params":{"activity":"updating sampler"}}`.
Supported activities are `waiting`, `materializing`, `evaluating`, `updating sampler`,
`saving checkpoint`, and `shutdown`. Notifications do not extend the request
timeout. Without notifications, a process sampler reports “waiting for sampler
response”; GammaBoard does not infer Python training activity from timing.

Process failures include the executable, working directory, operation, worker,
run/task, exit status, and a bounded stderr excerpt. `stderr_log` points to the
process's stderr log (oversized lines are omitted) under `resources/logs/processes/`. Credentials are
redacted from diagnostics and logs; command arguments and request payloads are
not included. Python traceback output on stderr remains separate from the framed
JSON-RPC protocol on stdout.

The optional MadNIS end-to-end test accepts `GAMMABOARD_MADNIS_STATE_PATH` and
`GAMMABOARD_MADNIS_INTEGRAND` for a state generated by the installed GammaLoop
version. Enable it with `GAMMABOARD_RUN_MADNIS_E2E=1`; set
`GAMMABOARD_MADNIS_PYTHON` to its Python environment.
Resumed workers use the new deployment/launcher environment, so make runtime
credentials and shared resource paths available as for an ordinary launch.

### Synthetic workloads and queue benchmarks

The built-in `unit` evaluator and `naive_monte_carlo` sampler support optional,
seeded Gaussian timing models for evaluation, generation, result ingestion and
repeating training updates. Defaults introduce no delay. Set
`training_window_samples` to enable a strict repeating training barrier; zero
selects inference. Synthetic worker panels show requested/actual delay and update
counts. These engines are available in ordinary builds.

The [queue benchmark suite](benchmarks/queue/README.md) covers 1, 4, 16 and 64
evaluators with nominal deployment compute ceilings from 1,000 to 2,000,000
samples/s. It supports generated run cards, isolated local runs, and paired A/B
measurements using prebuilt binaries. The default 16-case suite has a five-minute
wall-time budget, including paired A/B runs. Its burst cases alternate fast
generation with 0.5-second training updates:

```sh
just benchmark-queue run --binary target/release/gammaboard --smoke --output /tmp/queue-smoke
```

Use the timing tables instead of the former synthetic millisecond-delay fields.
