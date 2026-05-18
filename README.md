# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the
shared control plane. The dashboard shows runs, task output, nodes, performance,
logs, and active runtime settings.

## Start Here

Use the self-contained docs quickstart for setup and operator workflows:

- [docs/quickstart.md](docs/quickstart.md)
- [docs/README.md](docs/README.md)

Minimal local launch from the repo root:

```bash
./gammaboard \
  --runtime-config ops/local/config/runtime.toml \
  deploy run \
  --deploy-config ops/local/config/deploy.toml
```

Minimal ITPhlies release launch:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  --runtime-config ops/itphlies/config/runtime.toml \
  deploy run \
  --deploy-config ops/itphlies/config/deploy.toml
```

For UBELIX Slurm/Apptainer operation, use [ops/ubelix/README.md](ops/ubelix/README.md).

If you use Nix, the repository flake provides a development/operator shell with
Rust, Node.js, PostgreSQL, nginx, Apptainer, and helper tools. It is not the
deployment mechanism; production runs use the `gammaboard` binary, ops configs,
and, on UBELIX, Apptainer/Slurm images.

## Core Commands

- `gammaboard deploy run`: supervise local Postgres, backend API, and nginx/frontend in one foreground process.
- `gammaboard run`: create, list, pause, clone, remove, and append tasks to runs.
- `gammaboard node`: run workers, list nodes, assign roles, unassign, and request shutdown.
- `gammaboard db`: manage the local PostgreSQL instance used by the active runtime config.
- `gammaboard server`: run only the backend API for API-only/manual setups.

The repo-root `./gammaboard` helper builds the current CLI before forwarding
arguments to it. It uses `dev-optim` by default, `GAMMABOARD_PROFILE=release`
for release builds, and also builds the dashboard frontend for `deploy run`.

## Model

- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- Run names are human-facing and not unique. Ambiguous CLI name references fail.
- Node identity is a human-facing `name` plus a live-process `uuid`.
- Process evaluators and samplers are external commands speaking `gammaboard-jsonrpc-v1`.

## Development

```bash
cargo test -q
just test-e2e
```

Build without the heavy GammaLoop dependency:

```bash
cargo build --no-default-features
```
