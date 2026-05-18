# Quickstart

This page is the self-contained operator entry point for Gammaboard. It assumes
you are running commands from the repository root unless stated otherwise.

## Prerequisites

- Rust
- PostgreSQL 16 tools: `initdb`, `pg_ctl`, `postgres`, `psql`
- `sqlx` CLI:

```bash
cargo install sqlx-cli --no-default-features --features postgres
```

- Node.js and npm for building the dashboard frontend
- nginx for `gammaboard deploy run`

Optional Nix users can enter the repository flake dev shell to get these
operator tools from one environment:

```bash
nix develop
```

The flake is only a development/operator shell. It is not required at runtime
and is not the deployment mechanism. Deployments run the `gammaboard` binary
with the selected ops configs; UBELIX additionally uses Slurm and Apptainer
images.

## Start The Dashboard

Local development:

```bash
./gammaboard \
  --runtime-config ops/local/config/runtime.toml \
  deploy run \
  --deploy-config ops/local/config/deploy.toml
```

This builds the current Rust CLI, rebuilds the dashboard frontend, starts local
Postgres, starts the backend API, and serves the dashboard through nginx at
`http://localhost:8080`. Stop the stack with `Ctrl-C`.

ITPhlies release deployment:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  --runtime-config ops/itphlies/config/runtime.toml \
  deploy run \
  --deploy-config ops/itphlies/config/deploy.toml
```

Open `http://itphlies:8080` on the LAN, or tunnel from a workstation:

```bash
ssh -N -L 8080:127.0.0.1:8080 ITPhliesTails
```

UBELIX uses Slurm and Apptainer helpers. See the UBELIX operator workflow in
`ops/ubelix/README.md` when working from a full repository checkout.

## Helper Binary

The repo-root `./gammaboard` helper builds the current CLI before forwarding
arguments to it. It uses the `dev-optim` Cargo profile by default.

```bash
GAMMABOARD_PROFILE=release ./gammaboard --help
GAMMABOARD_PROFILE=debug ./gammaboard --help
```

When the forwarded command is `deploy run`, the helper also builds the
dashboard frontend.

## Isolated Instances

Use a port offset for a second local or ITPhlies instance:

```bash
./gammaboard \
  --runtime-config ops/local/config/runtime.toml \
  deploy run \
  --deploy-config ops/local/config/deploy.toml \
  --port-offset 1
```

`--port-offset 1` shifts frontend/API/Postgres from `8080/4000/5400` to
`8081/4001/5401` and suffixes local Postgres state paths.

## Create And Run Work

Create a run from a template:

```bash
./gammaboard run add resources/templates/runs/gammaloop.toml
```

Start local workers and assign them:

```bash
./gammaboard node auto-run 2
./gammaboard node assign w-1 sampler-aggregator gammaloop_tth
./gammaboard node assign w-2 evaluator gammaloop_tth
```

Inspect and pause:

```bash
./gammaboard run list
./gammaboard run task list gammaloop_tth
./gammaboard run pause gammaloop_tth
```

Stop nodes:

```bash
./gammaboard node stop -a
```

## Common Commands

Run commands:

```bash
./gammaboard run list [RUN_NAME]
./gammaboard run pause <RUN>
./gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
./gammaboard run task add <RUN> <TASK_FILE.toml>
./gammaboard run task remove <RUN> <TASK_ID>
./gammaboard run remove <RUN>
```

Node commands:

```bash
./gammaboard node list
./gammaboard node run --name <NODE_NAME>
./gammaboard node auto-run <COUNT>
./gammaboard node assign <NODE_NAME> <ROLE> <RUN>
./gammaboard node unassign <NODE_NAME>
./gammaboard node stop <NODE_NAME>
```

Database commands:

```bash
./gammaboard db status
./gammaboard db start
./gammaboard db reset --yes
```

## Manual Build And Deploy

Use the built binary directly when you do not want the repo-root helper.

Local dev profile:

```bash
cd dashboard
npm ci
npm run build
cd ..
cargo build --profile dev-optim
./target/dev-optim/gammaboard \
  --runtime-config ops/local/config/runtime.toml \
  deploy run \
  --deploy-config ops/local/config/deploy.toml
```

ITPhlies release profile:

```bash
cd dashboard
npm ci
npm run build
cd ..
cargo build --release
./target/release/gammaboard \
  --runtime-config ops/itphlies/config/runtime.toml \
  deploy run \
  --deploy-config ops/itphlies/config/deploy.toml
```

Useful deploy options:

- `--port-offset <N>` offsets frontend, API, and Postgres ports.
- `--api-port <PORT>` overrides the private backend API port for one launch.
- `--database-url`, `--postgres-data-dir`, `--postgres-socket-dir`, and
  `--postgres-log-file` isolate runtime state without editing TOML files.

## Where Things Live

The default resources root is `resources/`.

```text
resources/
  runtimes/          Process evaluator/sampler runtime projects and images
  states/            Shared model, integrator, and checkpoint state
  templates/         Run, task, and node launch TOML templates
```

Deployment artifacts are separate:

```text
artifacts/           Build inputs, compiled binaries, package caches
db/                  Local Postgres state
images/              Deployment/service images such as gammaboard.sif and gammaloop.sif
logs/                Postgres, Slurm, and deployment logs
runtime/             Runtime sockets, pids, and transient control files
```

The dashboard Settings tab shows the active config paths, resource root, and
Postgres paths.

## Testing

```bash
cargo test -q
just test-e2e
```

`just test-e2e` runs the ignored full-stack CLI tests. For serial debugging:

```bash
cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1
```

The process protocol benchmark is ignored by default:

```bash
cargo test -q process_evaluator_eval_batch_protocol_benchmark -- --ignored --nocapture
```

## GammaLoop Feature

GammaLoop support is behind the default `gammaloop` Cargo feature. Build without
the heavy GammaLoop dependency with:

```bash
cargo build --no-default-features
```

In that build, `evaluator.kind = "gammaloop"` and HwU histogram export return
explicit unsupported-feature errors.

## Next Pages

- [config.md](config.md): runtime, server, deploy, run, task, and node config.
- [deployment.md](deployment.md): shared deploy model, profiles, paths, images, and ports.
- [operations.md](operations.md): auth, node/run lifecycle, logs, and recovery.
- [process-runtime.md](process-runtime.md): external process evaluator/sampler protocol.
- [frontend.md](frontend.md): dashboard architecture and panel data flow.
