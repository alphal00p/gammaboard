# Deployment

A deployment consists of PostgreSQL, the `gammaboard` service and workers. The service
coordinates work and hosts the HTTP API and optional dashboard on one port. Workers
use PostgreSQL directly. The lightweight `gb` client manages the service remotely
and provides local startup and maintenance commands; see the [CLI reference](local-cli.md).

## Build features

The default build includes `gammaloop`, which enables `symbolica`. Feature flags
select computation backends; the persisted model and API protocol are shared.

```bash
cargo build --release --workspace --bins
# Service with Symbolica but without native GammaLoop:
cargo build --release --no-default-features --features symbolica
# Install only the lightweight client:
cargo install --path crates/gb
```

Use `--no-default-features` for a service without native physics backends. The
independent client has no SQLx, Symbolica or GammaLoop dependency.

In a checkout, `./gb` builds the client and invokes `./gammaboard` for local runtime
operations. These launchers default to `dev-optim`; set `GAMMABOARD_PROFILE=release`
for release builds. Prepared binaries avoid build work at invocation time.

For an installed client, service discovery checks `GAMMABOARD_SERVICE_BINARY`, then
a sibling `gammaboard`, then `PATH`. An invalid explicit override fails. Remote
management needs only `gb`; local start/restart and `gb local` also need the service
executable. Use absolute binary paths on cluster hosts.

Build optional dashboard assets with `npm ci` and `npm run build` in `dashboard/`.
Set `server.dashboard_dir` to the resulting `build` directory. Missing assets do
not block API startup; `gb start --api-only` explicitly disables static serving.

## Instance configuration

Use the checkout's `gammaboard.toml` or copy the
[instance example](../ops/local/instance.example.toml). Select it with
`--instance FILE`. Paths resolve relative to the file, so adjust them when copying
it to another directory. The file and its explicit paths identify the deployment;
`name` is a label. There is no instance registry.

| Setting | Purpose |
| --- | --- |
| `[paths]` | State, logs, temporary files, templates and resource roots |
| `[database]` | Managed PostgreSQL host/port/name/data/socket paths, or an external URL |
| `[database.tuning]` | PostgreSQL capacity and performance settings |
| `[server]` | Host, `api_port`, dashboard path, browser origins, authentication and local-launch policy |
| `[logging]` | Tracing and persisted logs |
| `[shutdown]` | Worker checkpoint/drain deadlines |
| `externally_managed` | Gate local bootstrap and computation |

Each concurrent instance needs separate state/data/log/temp directories and server
and managed-database ports. Locks prevent competing service owners. Managed
PostgreSQL is provisioned by `gb start`; an external database must be available
with the supported schema. Use `gb local db migrate` for supported offline migrations.

```bash
gb --instance /srv/board/instance.toml local config validate
gb --instance /srv/board/instance.toml start --foreground
```

A process manager can own the foreground command. Startup snapshots and
`status.json` live under `paths.state_dir`; service and PostgreSQL logs are
`paths.log_dir/service.log` and `paths.log_dir/postgres.log`. Pending exports and
shutdown requests live in PostgreSQL and survive service restart.

## Authentication and browser access

Set `[server.auth]` with `admin_password_hash`, `session_secret` and optional
`session_version`. Generate a hash with `gammaboard auth hash-password
--password-stdin`. Keep credential files private. Changing the session secret or
version invalidates existing tokens.

The dashboard uses a session cookie; CLI clients obtain a bearer token with
`gb auth login --password-stdin --output TOKEN_FILE`. Use HTTPS outside a trusted
network, usually through a site reverse proxy. Authenticated operators can launch
configured evaluator commands on the deployment; passwordless instances belong on
trusted networks.

Configure the exact origin shown in the browser address bar. For an SSH forward
from local port 39491 to service port 4000:

```toml
[server]
allowed_origins = ["http://localhost:4000", "http://localhost:39491"]
```

The list replaces defaults. To add an origin for one startup, use the repeatable
`--allowed-origin` flag on `start` or `restart`:

```bash
gb --instance /srv/board/instance.toml restart --allowed-origin http://localhost:39491
```

The flag does not edit the file. Origins accept HTTP(S) scheme, host and optional
port; wildcards, paths, credentials and implicit trust of forwarded headers are
unsupported. For Vite development, allow its actual origin too; see
[dashboard development](../dashboard/README.md).

## HPC

Set `externally_managed = true` and use explicit paths on shared storage. The
owner starts services and workers inside allocations with `--allow-local-processes`.
The override applies to that invocation; it does not change the API's launch policy.

The [Slurm/Apptainer helper](../ops/slurm/README.md) owns allocations, worker
submission/reconciliation and scheduler cleanup. It communicates through the
[HTTP launch protocol](cluster-launches.md). GammaBoard itself has no Slurm backend.
Ordinary users manage runs through `gb` and the published API endpoint; worker
jobs receive their own frozen database configuration.

`gb stop` requests application shutdown. The helper also verifies allocation
termination and supports forced cleanup when the service is unavailable or queued.
Site examples: [Slurm/UBELIX](../ops/slurm/examples/ubelix.toml) and
[ITPhlies](../ops/itphlies/README.md).
