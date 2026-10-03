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

Run lifecycle status counts only workers with live leases. Expired worker rows
remain available for history but do not keep a child or its campaign marked
running. In Campaign Sub-runs, a selected child shows `starting` until its sampler
is active, and `waiting` if it has no live sampler assignment.

Common commands:

```bash
gammaboard node run --name w-1
gammaboard node start-local 2
```

Dashboard node-start requests go through a generic launch-request queue. Local
deployments may resolve them by spawning child node processes. UBELIX resolves
them by submitting Slurm worker jobs.

New launch names use the first unused positive suffix for their prefix, checking
all persistent node records, including expired workers. Redeploying does not
reset names to `local-1`; this preserves identity and log history. Deploy with
`--resume-workers` to restart saved workers with their existing names.

The node list shows live leases only. A launch request is fulfilled once every
requested worker has connected at least once, even if some have since exited.
Only live workers and workers still awaiting their first connection for the
current launch reserve connection capacity. An expired worker's historical
launch must not consume capacity indefinitely.

## Run Lifecycle

Runs are created from TOML templates or custom TOML. Run names are human-facing
and not unique; ambiguous CLI name references fail.

Cloning starts from a persisted snapshot, not from in-memory worker state.
Workers belong to a root run's pool. Controllers alone choose child placements;
this rule is shared by campaigns, parameter scans, and hyperparameter tuning.
Assigning to any child (including nested descendants) resolves to the root pool,
and the CLI/API response reports that root. Assigning within the same pool and
role preserves the existing placement; it does not pin a child or restart work.
A transfer to another family changes pool ownership. Explicit unassignment or
node shutdown removes membership, so the old controller cannot reclaim the node.

Pausing any member of a run family pauses the root pool. Workers checkpoint and
become idle while retaining membership. `run resume RUN` / `node auto-assign RUN`
first resume retained members, then add eligible idle workers up to the requested
limit. The evaluator limit applies to newly added workers, not retained members.
Finished children return capacity to their immediate controller; exhausted root
runs release their pool. Controllers leave surplus workers parked at the parent,
and multiple samplers can belong to a pool while execution remains exclusive per
child. The dashboard manages worker pools on root pages and shows child placement
as an allocation by the parent. Node JSON includes pool, desired, and current
assignments separately.

Removing a run first unassigns its whole tree and waits for live workers to drain,
then deletes the records. If controller activity or draining exceeds 60 seconds,
removal fails and retains the records; retry after workers finish. This prevents
checkpoint and queue writes from racing with deletion.

Dashboard deletion runs in the background because removing a large history can
take longer than a proxy's HTTP timeout. `DELETE /api/runs/:id` returns HTTP 202
with an `operation_id`; poll `GET /api/run-removals/:operation_id` until `status`
is `completed` or `failed` (with an `error`). Both endpoints require dashboard
authentication. Repeated submissions for the same run share its active operation.
Closing the browser does not cancel deletion; the dashboard only reports success
after the transaction commits and keeps errors visible until dismissed. The CLI
continues to wait synchronously for deletion.

Operation status is held in server memory, with completed entries eligible for
cleanup after one hour. If the server restarts or status has expired, refresh the
run list before retrying: a committed deletion remains deleted, while an
interrupted transaction rolls back.

## Logs

Runtime logs are persisted to Postgres and exposed in the dashboard Logs tab.
Process worker stderr is normal log output. Process worker stdout is reserved
for framed `gammaboard-jsonrpc-v4`; wrappers should redirect accidental prints
to stderr. See [process-runtime.md](process-runtime.md).

Useful locations:

- Dashboard Logs tab: run-scoped persisted logs.
- `resources/logs/nodes` or profile-specific node logs: local node process logs.
- `resources/db/logfile` (or its configured equivalent): managed PostgreSQL log.
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

## Database Backup And Restore

Create a compressed backup of the database selected by the runtime config:

```bash
gammaboard db backup
gammaboard db backup --output /safe/path/gammaboard.dump
```

The default output is `backups/gammaboard-<timestamp>.dump`. To restore, first
stop the GammaBoard server and workers so they cannot write concurrently, then
run:

```bash
gammaboard db restore /safe/path/gammaboard.dump
```

Restore replaces the configured database contents and applies pending
migrations afterward, so a backup from the immediately preceding schema can be
used during an upgrade. In scripts, acknowledge replacement with `--yes`.

## Capacity Planning

Each `node run` process uses up to two PostgreSQL connections for leases and control
traffic, including controller exclusion during leadership changes. Its active
evaluator role is capped at two additional connections; a sampler role at six.
Budget four connections per live node, four extra per sampler, plus
the server and occasional CLI commands. The default local PostgreSQL limit is
128; reserve at least 16 connections for the server, maintenance, and operator
commands before choosing a worker count.

Managed launch and resume requests enforce this budget before reserving workers:
`floor((max_connections - PostgreSQL reserved connections - 16 - 4 × samplers) / 4)`.
With 128 connections and the usual three superuser reservations, this allows
27 workers without a sampler, or 26 including one sampler. Sampler assignment
checks its additional reservation under the same lock as launch admission.
Live idle workers and pending launches count toward that limit
because assigning them later creates another connection pool. Stop unused
workers to free capacity; increasing PostgreSQL's limit requires a database
restart. Directly started `node run` processes and unrelated database clients
still require operator budgeting. Pools release spare idle connections after
30 seconds.

For managed PostgreSQL, change `local_postgres.max_connections` in the runtime
config, **not** the server config or the database's generated `postgresql.conf`.
GammaBoard passes this setting to PostgreSQL on its command line. The default
runtime config path is `ops/local/config/runtime.toml`; if it does not exist,
embedded defaults apply. Create it with, for example:

```toml
[local_postgres]
max_connections = 512

[resources]
roots = ["resources"]
```

Keep the resource root explicit when using older binaries: they treated an
omitted `[resources]` section as an empty search path, changing relative state
paths to resolve from the process working directory. Current binaries default
omitted roots to `["resources"]`; an explicit `roots = []` opts out.
Alternatively, put the TOML
elsewhere and pass `./gammaboard --runtime-config /path/to/runtime.toml deploy`
alongside your usual deployment options. Stop the deployment and restart its
PostgreSQL instance before redeploying: starting against an already running
database does not change this setting. `SHOW max_connections;` confirms the
active value. With three PostgreSQL reserved connections and one sampler,
limits of 256, 512, and 1024 allow 58, 122, and 250 workers respectively.

For large materialized/training workloads, the measured high-throughput profile
uses `shared_buffers = "4GB"` in `[local_postgres]`, alongside the default six
sampler connections and four inserts. This increases PostgreSQL's cache allocation;
the general-purpose default remains 256MB for smaller deployments. It also requires
a PostgreSQL restart. See [the throughput experiments](performance-development.md#sampler-io-tuning--2026-09-30)
for the measured scope and tradeoffs.

## Queue Recovery and Upgrades

Sampler recovery compares the saved and loaded checkpoint through the same Rust
checkpoint type. PostgreSQL JSONB can represent a large floating-point Jacobian
as an integer; this alone must not trigger a checkpoint-change retry. The row
lock, exact comparison of decoded state, and queued-sample consistency checks
still reject genuinely stale or inconsistent recovery attempts.

Evaluators retain claimed batches and computed results across database errors.
Each claim has a unique token: retries acknowledge the same claim or result,
and reassignment fences submissions from its previous owner. Once per second,
an evaluator releases its own claims that it no longer tracks in memory. Claim
age alone never revokes a long evaluation. Expired workers are still recovered
through the existing lease mechanism.

Sampling consumes results belonging to the current task only. Retained retry
history from a completed training task cannot enter the next sampling task.
Performance Diagnostics shows the first unfinished batch, its worker and claim
age, and result-fetch duration. "Result Fetch Slot Occupancy"
includes a finished fetch waiting for the next sampler tick; it does not measure
database utilization or the number of buffered results.

When installing the claim-token migration (`202609230001`), stop the old server
and all old workers, apply migrations, and restart them with the new binary.
Old evaluator binaries do not honor claim tokens, so a mixed-version rollout
does not provide the ownership guarantees. The migration preserves existing
batches and results; old claims become reclaimable when their workers expire.

These changes prevent new cross-task contamination. They do not repair sampling
accumulators or checkpoints already affected by replayed training results.
Restart such sampling stages with a fresh accumulator and a verified training
snapshot, including any external model files. Do not bypass consistency checks
or edit only the sample counters.

## Database History

Performance snapshots default to every two seconds. One evaluator therefore
creates 43,200 history rows per day. Monitor database size for multi-day
campaigns and use normal PostgreSQL operations when history must be managed.

Before increasing a deployment, measure its intended configuration with the
CLI. Do not extrapolate storage growth from the short CPU benchmark presets:

```bash
gammaboard --json run performance RUN --duration 60s --interval 1s
```

Increase workers only while the queue stays bounded and database latency remains
stable. For long-running deployments, separately inspect PostgreSQL database
size and history retention. See [benchmarking.md](benchmarking.md) for isolated,
bounded throughput comparisons using the same CLI measurement contract.

## Reading Activity and Fetch Metrics

Overview and Graphs show evaluator compute/I/O and sampler compute/I/O over one
selected window. I/O active is the fraction of time with at least one work-related
operation running, including database waits. Completed operations awaiting
collection no longer count; the retained-slot-occupancy percentages were removed.
Compute and I/O may overlap. Neither is OS CPU or database utilization.

In Diagnostics, `Result fetch` measures operation latency and evaluator
`Exposed fetch wait (µs/sample)` measures unhidden latency on successful batches.
Persistence durations also end at completion rather than collection. Operation
timings overlap and are not an additive breakdown. See
[frontend.md](frontend.md#measurement-semantics) for coverage and resource scope.
