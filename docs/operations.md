# Operations

This page covers day-to-day dashboard operation. Environment-specific command
wrappers live in `ops/*/README.md`.

## Authentication

If `[auth]` is configured in the server config, the dashboard/API requires the
single admin password and uses a signed session cookie. If `[auth]` is omitted,
the dashboard/API is passwordless. There are no users or roles: anyone with
access is an administrator.

Run TOML can launch configured child-process commands. Treat dashboard access,
the API, and CLI access as trusted-operator access; do not give them to
untrusted users, public web clients, or autonomous agents without an external
approval boundary.

Generate an admin password hash:

```bash
gammaboard auth hash-password
```

Relevant server config keys:

- `auth.admin_password_hash`
- `auth.session_secret`
- `secure_cookie`
- `allowed_origins`

UBELIX helpers accept `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD` for
admin-protected shutdown and worker-management operations.

## Node Lifecycle

Nodes are identified by persistent `name` plus live-process `uuid`. Desired and
current assignments are stored in Postgres. A stale UUID lease is replaced when
the same node name starts again.

Common commands:

```bash
gammaboard node run --name w-1
gammaboard node start-local 2
```

Dashboard node-start requests go through a generic launch-request queue. Local
deployments may resolve them by spawning child node processes. UBELIX resolves
them by submitting Slurm worker jobs.

## Run Lifecycle

Runs are created from TOML templates or custom TOML. Run names are human-facing
and not unique; ambiguous CLI name references fail.

Cloning starts from a persisted snapshot, not from in-memory worker state.
Removing a run clears worker assignments immediately and prevents new work from
being claimed for that run.

## Logs

Runtime logs are persisted to Postgres and exposed in the dashboard Logs tab.
Process worker stderr is normal log output. Process worker stdout is reserved
for framed `gammaboard-jsonrpc-v2`; wrappers should redirect accidental prints
to stderr. See [process-runtime.md](process-runtime.md).

Useful locations:

- Dashboard Logs tab: run-scoped persisted logs.
- `resources/logs/nodes` or profile-specific node logs: local node process logs.
- `logs/postgres`: local Postgres logs.
- `logs/slurm`: UBELIX Slurm stdout/stderr.

## Failure Recovery

- `Address already in use`: free the conflicting frontend, API, or Postgres port
  or restart with a port offset.
- DB start failure: inspect the profile-specific Postgres log.
- Worker exits: inspect persisted runtime logs first, then node/Slurm stderr.
- Repeated task errors: inspect task error text and process stderr; failed
  batches are retried according to queue policy, while sampler construction
  errors can fail the task.
- Stale workers: stop/unassign from the dashboard or use the profile helper
  command, then start fresh workers.
## Capacity Planning

Each `node run` process keeps one PostgreSQL connection for leases and control
traffic. Its active evaluator or sampler role is capped at two additional
connections, so plan for at most three database connections per live node plus
the server and occasional CLI commands. The default local PostgreSQL limit is
128; reserve at least 16 connections for the server, maintenance, and operator
commands before choosing a worker count.

Performance snapshots default to every two seconds. One evaluator therefore
creates 43,200 history rows per day. Monitor database size for multi-day
campaigns and use normal PostgreSQL operations when history must be managed.

Before increasing a deployment, measure its intended configuration rather than
extrapolating from a smoke test. This self-cleaning local benchmark starts an
isolated stack, runs one sampler plus the requested evaluators, and reports the
relevant rates and maxima. Its dependency-free workload deliberately throttles
worker ticks to keep the default ten-minute run small enough for a local
PostgreSQL instance:

```bash
nix develop --command scripts/benchmark_campaign.sh --workers 4 --duration-seconds 600
```

Set `GAMMABOARD_BENCHMARK_DATABASE_URL` for a non-default local database URL.
Increase workers only while the queue stays bounded and database latency
remains stable.
