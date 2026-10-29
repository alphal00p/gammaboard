# Deployment

`gammaboard deploy` supervises the API, nginx/dashboard, managed PostgreSQL, and
local worker launch requests. Ctrl-C drains workers and stops the stack.
Profile commands: [local](../ops/local/README.md),
[ITPhlies](../ops/itphlies/README.md), [UBELIX/Slurm](../ops/ubelix/README.md).

## Configuration and ports

Runtime config controls database connection, resource roots, logging, and local
PostgreSQL. The checkout uses `ops/local/config/runtime.toml` when present;
omitted settings use embedded defaults. Select another file with
`--runtime-config PATH`. Common overrides are `--database-url URL` and
`--resource-root PATH`. Relative state paths resolve through `resources.roots`.

Server config controls hosting; pass `deploy --server-config PATH`:

```toml
api_host = "127.0.0.1"
api_port = 4000
allowed_origins = ["http://localhost:8080"]
allow_local_node_spawn = true
secure_cookie = false

[frontend]
host = "127.0.0.1"
port = 8080

[database]
ensure_started = true
```

Set `frontend.build_dir` for a separately installed dashboard. Set
`allow_local_node_spawn = false` when an external launcher handles worker requests.
Default frontend/API/PostgreSQL ports are **8080/4000/5400**.
`--port-offset 1` shifts them to **8081/4001/5401** and isolates managed state paths;
use the same offset on every command for that instance.

For a tunnel or alternate hostname, allow the origin shown in the browser:

```bash
./gammaboard deploy --allowed-origin http://localhost:39491
```

This option is repeatable. Persistent origins belong in `allowed_origins`.
Configured local origin ports shift with the port offset; CLI origins are added
as written. Set `GAMMABOARD_FRONTEND_BASE=/board/` when building for a URL subpath.

## Access

Anyone with access can launch configured process commands. Passwordless control
is for trusted networks. To require a single administrator password, run
`gammaboard auth hash-password`, then add to a private server config:

```toml
[auth]
admin_password_hash = "<Argon2 hash>"
session_secret = "<random secret>"
```

Use HTTPS and top-level `secure_cookie = true` for secure cookies. There are no
per-user roles. Increasing `auth.session_version` invalidates existing sessions.
`--postgres-trusted-network` permits passwordless database access from directly
connected networks; every reachable user/device must be trusted.

## Capacity and storage

Budget **4 PostgreSQL connections per worker, plus 4 per sampler**, and at least
16 for the server/maintenance. Managed launches enforce available capacity;
manual workers and unrelated clients also consume connections. Stop unused workers.
Embedded PostgreSQL defaults allow 128 connections; profiles can override this.
Change the active runtime config and restart PostgreSQL:

```toml
[local_postgres]
max_connections = 512
# For a large payload workload on a sufficiently sized host:
# shared_buffers = "4GB"
```

Increasing sampler `io_threads` can help materialized/training workloads; allocate
several physical cores to that worker too. Scaling is workload-dependent. Worker
CPU allowance/Slurm allocation controls cores, not `io_threads`. Reserve separate
capacity for PostgreSQL and use [benchmarks](benchmarking.md) before scaling.

Keep states, process environments, and checkpoint files accessible at the same
paths on all relevant workers. A service image contains GammaBoard; process
runtime images contain task dependencies. Rebuild workers with the matching
[process protocol](../process_api/README.md). GPU containers need `--nv` and
matching binds. Normal builds include Symbolica licensing; builds made with
`NO_SYMBOLICA_OEM_LICENSE=1` require a regular runtime license.

For external launch leases, per-worker receipts, and deployment/build fencing, see
the [cluster launch contract](cluster-launches.md). UBELIX runs from a Git checkout
with settings in `ops/ubelix/local.toml`. Builds replace fixed service images while
the workspace is stopped; deployments retain image digests and configuration/script
snapshots. See its [operator guide](../ops/ubelix/README.md).

## Upgrading this revision

Use the [compatibility policy](compatibility.md) to match binaries, SDKs, generated
physics states, and persisted formats. Incompatible models require a fresh database;
there is no definition/state converter. Preserve the old deployment and backup,
then recreate runs from current validated templates. See
[backup and recovery](operations.md#backup-and-recovery) for operator commands.
