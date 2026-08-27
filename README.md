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
passwordless control on loopback. Security configuration is optional. A
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

## Process Runtimes

External evaluators, samplers, transforms, and materializers speak framed
JSON-RPC over stdin/stdout. The protocol is in
[docs/process-runtime.md](docs/process-runtime.md); the Python helpers and
working runtimes are in [process_api](process_api).

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
