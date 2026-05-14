# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Quickstart

Local development from the repo root:

```bash
just deploy local dev
```

This builds the dashboard when needed, starts local Postgres, starts the backend, and serves the dashboard through nginx at `http://localhost:8080`. Stop with `Ctrl-C`.

ITPhlies release deploy from the repo root on ITPhlies:

```bash
just deploy itphlies release
```

Open `http://itphlies:8080` on the LAN, or tunnel:

```bash
ssh -N -L 8080:127.0.0.1:8080 ITPhliesTails
```

Run an isolated second instance with a port offset:

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
- `just` for checked-in wrapper commands

## Common Workflow

Create a run:

```bash
gammaboard run add resources/templates/runs/gammaloop.toml
```

Start local workers and assign them:

```bash
gammaboard node auto-run 2
gammaboard node assign w-1 sampler-aggregator gammaloop_tth
gammaboard node assign w-2 evaluator gammaloop_tth
```

Inspect, pause, and stop:

```bash
gammaboard run list
gammaboard run task list gammaloop_tth
gammaboard run pause gammaloop_tth
gammaboard node stop -a
```

Useful run/node commands:

```bash
gammaboard run list [RUN_NAME]
gammaboard run pause <RUN>
gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
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

## Manual Deploy

Use the deploy helper directly when you do not want the `just` wrappers.

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

- `--port-offset <N>` adds `N` to configured frontend/API/Postgres ports and suffixes local Postgres state paths.
- `--api-port <PORT>` overrides the private backend API port for one launch.
- Global runtime overrides such as `--database-url`, `--postgres-data-dir`, `--postgres-socket-dir`, and `--postgres-log-file` can isolate instances without editing TOML files.

## Core Model

- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- Run names are human-facing and not unique. CLI name references fail when ambiguous.
- Node identity is a human-facing `name` plus a live-process `uuid`; desired/current assignments live on `nodes`.
- Process evaluators and samplers are external commands speaking `gammaboard-jsonrpc-v1`.

## Documentation

- [docs/config.md](docs/config.md): deploy/runtime/server/run/task/node config reference.
- [docs/process-runtime.md](docs/process-runtime.md): external process evaluator/sampler protocol.
- [process_api/README.md](process_api/README.md): process API examples and wrappers.
- [ops/ubelix/README.md](ops/ubelix/README.md): UBELIX operator workflow.
- [ops/itphlies/README.md](ops/itphlies/README.md): ITPhlies notes.

## Useful Local Commands

```bash
gammaboard run pause -a
gammaboard node stop -a
cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1
cargo test -q process_evaluator_eval_batch_protocol_benchmark -- --ignored --nocapture
```

GammaLoop support is behind the default `gammaloop` Cargo feature. Build without the heavy GammaLoop dependency with:

```bash
cargo build --no-default-features
```

In that build, `evaluator.kind = "gammaloop"` and HwU histogram export return explicit unsupported-feature errors.
