# Deployment

A deployment consists of PostgreSQL, the `gammaboard` service and workers. The service
coordinates work and hosts the HTTP API and optional dashboard on one port. Workers
use PostgreSQL directly. The lightweight `gb` client manages the service remotely
and provides local startup and maintenance commands; see the [CLI reference](local-cli.md).

Use installed [system dependencies](quickstart.md) on `PATH`. `nix develop` is an
optional reproducible environment; startup does not require Nix.

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

The repository wrappers prepare the frontend for `start` and `restart`:

- Build missing or changed assets into `server.dashboard_dir` (default
  `dashboard/build`, relative to the instance file).
- Install locked JavaScript packages with `npm ci` when the dependency cache is
  missing or changes, including after a Node version change during a rebuild.
- Reuse a current build without requiring Node/npm or `node_modules`.
- Skip preparation for `--api-only`, `start db`, help and remote management.
- Respect `externally_managed`; local build/install work requires the explicit
  `--allow-local-processes` override.

Builds use content fingerprints of frontend sources, package files, Vite configuration,
`.env` files and frontend environment variables. Preparation is serialized per checkout.
A failed build leaves the previous assets intact and stops startup with an error.
Progress goes to stderr, preserving `--json` stdout.

Custom prebuilt directories with `index.html` are used unchanged unless previously
built by the wrapper. A new custom output directory must be empty. The wrapper
never installs system packages; missing Node/npm produce a readable error.

Installed binaries serve prebuilt assets without building them. To prepare assets
manually, run `npm ci` and `npm run build` in `dashboard/` and configure
`server.dashboard_dir`. Missing assets do not block the installed service's API;
`--api-only` explicitly disables static serving.

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

By default, `server.allow_loopback_origins = true` accepts HTTP(S) browser origins
on exactly `localhost`, `127.0.0.1`, or `[::1]` at any valid port. SSH/browser port
forwarding therefore works when the local port changes. This trusts pages served
by other local applications on the **browser's machine**; it does not verify an
SSH tunnel or replace authentication. Other hostnames, loopback aliases and
subdomains, `null` origins, and malformed origins are rejected unless an exact
valid origin is configured.

For a shared deployment, disable this convenience and list the exact browser origins:

```toml
[server]
allow_loopback_origins = false
allowed_origins = ["https://board.example.org", "http://localhost:39491"]
```

`allowed_origins` replaces the default exact list; when loopback trust is enabled,
its entries are additional trusted origins. An empty list is permitted only with
loopback trust enabled. Entries accept HTTP(S) scheme, host and optional port;
wildcards, paths, credentials and implicit trust of forwarded headers are unsupported.

After editing either origin setting in the deployment file, reload it without
restarting the service, runs or workers:

```bash
gb config reload --origins
```

This authenticated management operation reads the **service's deployment file**,
not a client-side file. It also works through a remote connection file or endpoint
and on externally managed deployments. It validates the file and atomically
replaces the policy used by CORS and browser request validation. Invalid settings
leave the previous policy active. Other configuration changes remain unapplied
until restart. In-flight requests are not canceled. `gb status` and dashboard
settings show the effective policy; a later `gb start` reuses the service when
configuration matches.

For a one-start addition, use the repeatable `--allowed-origin` flag:

```bash
gb --instance /srv/board/instance.toml restart --allowed-origin https://board.example.org
```

The flag does not edit the file. Reload replaces both origin settings from the
file, discarding these invocation-only additions.
Slurm deployments use frozen generated files: edit the source instance and use the
helper's [`reload-origins`](../ops/slurm/README.md#browser-origin-reload) command to
copy only browser policy into the control snapshot before invoking the API. For Vite development, the
default loopback policy covers its changing port; see
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
