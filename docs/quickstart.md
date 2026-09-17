# Quickstart

This page is the self-contained operator entry point for GammaBoard. It assumes
you are running commands from the repository root unless stated otherwise.

## Prerequisites

- Rust
- PostgreSQL 16 tools: `initdb`, `pg_ctl`, `postgres`, `psql`
- `sqlx` CLI:

```bash
cargo install sqlx-cli --no-default-features --features postgres
```

- Node.js and npm for building the dashboard frontend
- nginx for `gammaboard deploy`

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
./gammaboard deploy
```

This builds the current Rust CLI, builds the dashboard frontend when its sources
or build configuration changed, starts local Postgres and the backend API, and
serves the dashboard through nginx at `http://localhost:8080`. Use
`./gammaboard deploy --rebuild-frontend` to force a frontend rebuild. Stop the
stack with `Ctrl-C`.

ITPhlies release deployment:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  deploy \
  --server-config ops/itphlies/config/server.toml
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

When the forwarded command is `deploy`, the helper also ensures that the
dashboard frontend build is current. `--rebuild-frontend` is a helper option;
the built Rust binary expects an already-built frontend.

## Isolated Instances

Use a port offset for a second local or ITPhlies instance:

```bash
./gammaboard \
  --port-offset 1 \
  deploy
```

`--port-offset 1` shifts frontend/API/Postgres from `8080/4000/5400` to
`8081/4001/5401` and suffixes local Postgres state paths.

## Create And Run Work

Create a run from a template:

```bash
./gammaboard run create resources/templates/runs/installation-smoke.toml
```

Start local workers and assign them:

```bash
./gammaboard node start-local 2
./gammaboard node assign w-1 sampler_aggregator installation-smoke
./gammaboard node assign w-2 evaluator installation-smoke
```

Inspect and pause:

```bash
./gammaboard run list
./gammaboard run task list installation-smoke
./gammaboard run pause installation-smoke
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
./gammaboard run task append <RUN> <TASK_FILE.toml>
./gammaboard run task remove <RUN> <TASK_ID>
./gammaboard run remove <RUN>
```

Node commands:

```bash
./gammaboard node list
./gammaboard node run --name <NODE_NAME>
./gammaboard node start-local <COUNT>
./gammaboard node assign <NODE_NAME> <ROLE> <RUN>
./gammaboard node unassign <NODE_NAME>
./gammaboard node stop <NODE_NAME>
```

Database commands:

```bash
./gammaboard db status
./gammaboard db start
./gammaboard db backup
./gammaboard db restore backups/gammaboard-<timestamp>.dump
./gammaboard db reset --yes
```

`db backup` writes a compressed PostgreSQL archive and prints its path. Stop
GammaBoard servers and workers before `db restore`; restore replaces the
configured database, then applies any migrations newer than the archive. Pass
`--yes` for non-interactive restore.

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
  deploy
```

ITPhlies release profile:

```bash
cd dashboard
npm ci
npm run build
cd ..
cargo build --release
./target/release/gammaboard \
  deploy \
  --server-config ops/itphlies/config/server.toml
```

Useful deploy options:

- `--port-offset <N>` offsets frontend, API, and Postgres ports.
- `--api-port <PORT>` overrides the private backend API port for one launch.
- `--database-url` and `--resource-root` are the normal runtime overrides for
  manual or shared deployments.
- `--postgres-trusted-network` allows workers on directly connected networks to
  use passwordless PostgreSQL; keep the default loopback-only mode unless this
  is needed.

## Testing

```bash
cargo test -q
just test-e2e
```

`just test-e2e` starts the managed local PostgreSQL cluster, then runs the
ignored full-stack CLI tests with four test threads. The tests create and
migrate their own temporary databases, so an existing local `gammaboard_db`
does not affect them. Set
`GAMMABOARD_E2E_TEST_THREADS` to tune concurrency. For serial debugging:

```bash
cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1
```

The Apptainer E2E test builds and runs a container image. In the Nix development
shell, `apptainer` uses a compatibility wrapper when `/bin/true` is missing:
it supplies `/bin/true`, `/bin/sh`, and `/bin/bash` inside a private user/mount
namespace for both `build` and `exec`. The host filesystem is unchanged, and
files created in the namespace remain owned by the invoking host user. This
requires working unprivileged user namespaces; it does not bypass a host policy
that disables them. Hosts with `/bin/true` skip the namespace wrapper. If the
host has no `/etc/localtime`, the wrapper also disables that default bind using
[`APPTAINER_NO_MOUNT`](https://apptainer.org/docs/user/main/appendix.html),
preserving any existing exclusions and leaving the image's timezone in place.

The GammaLoop/MadNIS E2E test requires a generated state compatible with the
pinned GammaLoop revision and is separate:

```bash
just test-e2e-madnis
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
- [deployment.md](deployment.md): profiles, resource layout, images, and ports.
- [operations.md](operations.md): auth, node/run lifecycle, logs, and recovery.
- [process-runtime.md](process-runtime.md): external process evaluator/sampler protocol.
- [frontend.md](frontend.md): dashboard architecture and panel data flow.
