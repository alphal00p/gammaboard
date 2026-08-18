# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the
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

This local workflow intentionally uses public development credentials and
passwordless control on loopback. Security configuration is optional.

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

## Core Commands

- `gammaboard deploy`: supervise local Postgres, backend API, and nginx/frontend in one foreground process.
- `gammaboard run`: create, list, pause, clone, remove, and append tasks to runs.
- `gammaboard node`: run workers, list nodes, assign roles, unassign, and request shutdown.
- `gammaboard db`: manage the local PostgreSQL instance used by the active runtime config.
- `gammaboard server`: run only the backend API for API-only/manual setups.

The repo-root `./gammaboard` helper builds the current CLI before forwarding
arguments to it. It uses `dev-optim` by default, `GAMMABOARD_PROFILE=release`
for release builds, and also builds the dashboard frontend for `deploy`.
Set `GAMMABOARD_FRONTEND_BASE=/board/` when building the dashboard for a
reverse-proxy mount below a URL path instead of at `/`.

## Model

- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- The node supervisor leader advances task state and controller tasks in the control plane; sampler/evaluator assignments are reserved for compute work.
- Run names are human-facing and not unique. Ambiguous CLI name references fail.
- Node identity is a human-facing `name` plus a live-process `uuid`.
- Process evaluators, samplers, batch transforms, and materializers are external commands speaking `gammaboard-jsonrpc-v2`.

## Core Ideas

Gammaboard separates integration into a persisted control plane and stateless-ish
workers that can come and go.

- A run is the immutable top-level problem definition: domain, root evaluator,
  default runner settings, and the task queue.
- A task is one step on the run timeline, such as sampling, plotting, setting an
  accumulator, parameter scanning, or hyperparameter tuning.
- A snapshot records the stage state after a task: accumulator state, sampler
  state, evaluator config, and batch transform config. Later tasks restore from
  these snapshots instead of relying on in-memory handoff.
- A node is a live worker process registered in PostgreSQL by `name` plus
  process `uuid`. Nodes receive desired role assignments from the control plane:
  sampler aggregator, evaluator, or supervisor.
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
- Accumulators own observable semantics: scalar/vector/full-vector/GammaLoop
  state, error estimates, moments, projections, and panel-ready metrics.

The hot path is:

```text
sampler aggregator -> latent batch queue -> materializer -> batch transforms -> evaluator -> accumulator snapshot/training feedback
```

The run domain is authoritative throughout this path. Samplers produce points in
that domain, materializers and transforms must preserve a valid concrete batch,
and evaluator workers validate materialized/transformed batches before calling
the configured evaluator.

## Process API

Custom evaluators, samplers, and materializers can run as ordinary child
processes. GammaBoard sends framed JSON-RPC requests on stdin and reads
responses from stdout; stderr is used for logs. See
[docs/process-runtime.md](docs/process-runtime.md) for the full protocol.

For Python runtimes, install the small helper package:

```bash
pip install "gammaboard-process @ git+https://github.com/alphal00p/gammaboard.git@fdd59328814019a524a7838783efde8b42af3d50#subdirectory=process_api/python"
```

Minimal evaluator:

```python
from gammaboard_process import Evaluator, run_evaluator

class MyEvaluator(Evaluator):  # ABC inheritance is optional.
    def eval(self, xs_discrete, xs_continuous):
        return xs_continuous[:, 0]

run_evaluator(MyEvaluator)
```

Minimal sampler:

```python
from gammaboard_process import SampleBatch, Sampler, run_sampler

class MySampler(Sampler):  # ABC inheritance is optional.
    def sample_plan(self):
        return {"kind": "produce", "nr_samples": 1024}

    def produce_latent_batch(self, nr_samples):
        return SampleBatch(xs_discrete, xs_continuous, weights)

    def ingest_training_values(self, training_values):
        pass

    def snapshot(self):
        return {}

run_sampler(MySampler)
```

Minimal batch transform:

```python
from gammaboard_process import BatchTransform, TransformedBatch, run_batch_transform

class MyTransform(BatchTransform):  # ABC inheritance is optional.
    def transform_batch(self, xs_discrete, xs_continuous, weights):
        return TransformedBatch(xs_discrete, xs_continuous, weights)

run_batch_transform(MyTransform)
```

Minimal materializer:

```python
from gammaboard_process import MaterializedBatch, Materializer, run_materializer

class MyMaterializer(Materializer):  # ABC inheritance is optional.
    def materialize_batch(self, latent_batch):
        payload = latent_batch["payload"]
        return MaterializedBatch(xs_discrete, xs_continuous, weights)

run_materializer(MyMaterializer)
```

GammaBoard derives `discrete_cardinalities` and `continuous_dims` from the run
domain and passes them as keyword arguments together with the process config
`args`. Sampler restore may additionally implement
`from_snapshot(snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)`.
Examples live in
[process_api/examples](process_api/examples).

Log from a worker with `gammaboard_process.log("msg", level="info")`
(`trace|debug|info|warn|error`); plain `print()` is captured at info. Logs become
runtime logs with `source = "worker"`. See
[docs/process-runtime.md](docs/process-runtime.md#logging).

Use a process batch transform for process-side parametrizations that map
concrete sampled coordinates to evaluator coordinates:

```toml
[[task_queue.batch_transforms]]
kind = "process_batch_transform"
command = ["python", "-u", "-m", "my_runtime.transform"]
args = { scale = 1.0 }
```

Attach a process materializer to a sampler config only when the sampler emits a
latent representation that must be mapped before normal batch transforms:

```toml
[task_queue.sampler_aggregator.config]
kind = "process_sampler"
command = ["python", "-u", "-m", "my_runtime.sampler"]

[task_queue.sampler_aggregator.config.materializer]
kind = "process_materializer"
command = ["python", "-u", "-m", "my_runtime.materializer"]
args = { scale = 1.0 }
```

Some examples use generated local artifacts that are not committed. Build all
currently known optional process artifacts with:

```bash
just process-artifacts
```

To build optional artifacts, create all bundled example runs, and start two
local workers per run against the active runtime config:

```bash
just start-example-runs
```

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

Gammaboard is intended to be distributed under the MIT License.

Normal GammaBoard builds include OEM-licensed Symbolica activation, so users do
not need to obtain or configure a separate Symbolica license. Symbolica remains
subject to its own license terms: https://symbolica.io/license.html

Builds without the default `gammaloop` feature do not link GammaLoop, but
Gammaboard still depends directly on Symbolica for built-in Symbolica
evaluators. Developers who explicitly compile with
`NO_SYMBOLICA_OEM_LICENSE=1` must provide `SYMBOLICA_LICENSE` at runtime.
