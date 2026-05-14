# Operations

This page covers day-to-day dashboard operation. Environment-specific command
wrappers live in `ops/*/README.md`.

## Authentication

Read-only dashboard/API endpoints are open. Steering actions require admin
login and use a signed session cookie.

Generate an admin password hash:

```bash
gammaboard auth --password '<password>'
```

Relevant server config keys:

- `auth.admin_password_hash`
- `auth.session_secret`
- `auth.secure_cookie`
- `auth.allowed_origins`

UBELIX helpers accept `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD` for
admin-protected shutdown and worker-management operations.

## Node Lifecycle

Nodes are identified by persistent `name` plus live-process `uuid`. Desired and
current assignments are stored in Postgres. A stale UUID lease is replaced when
the same node name starts again.

Common commands:

```bash
gammaboard node run --name w-1
gammaboard node auto-run --prefix w --count 2
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
for framed `gammaboard-jsonrpc-v1`; wrappers should redirect accidental prints
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
