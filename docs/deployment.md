# Deployment

GammaBoard has two executables: `gb`, the lightweight HTTP client, and
`gammaboard`, the control/API service and worker runtime. The service owns local
lifecycle coordination and can serve dashboard files on its API port. Workers
continue using PostgreSQL directly. There is no separate nginx/supervisor stack.

## Build features

The default service build includes `gammaloop`, which also enables `symbolica`.
A reduced build can use `--no-default-features`; add `--features symbolica` for the
native tetrahedron example. Feature flags affect available computation backends,
not the persisted model or API protocol.

```bash
cargo build --release --workspace --bins
cargo build --release --no-default-features --features symbolica
cargo install --path crates/gb
```

The independently installed client does not depend on SQLx, Symbolica, GammaLoop,
or the main library. Local `gb start` finds a sibling `gammaboard` executable or
uses `GAMMABOARD_SERVICE_BINARY`. Run `npm ci` and `npm run build` in `dashboard/`
when dashboard assets are wanted; missing assets never block API startup.

## Configuration and ownership

Use an explicit instance TOML file with paths, database settings, one server port,
logging, and shutdown settings. Relative paths resolve against the file. A managed
database is provisioned by `start`; an external database must already be available
and have the supported schema. Separate instances require separate state/data
paths and ports. PostgreSQL remains a separate process from the control service.

```bash
gammaboard --instance /srv/board/instance.toml check-config
gammaboard --instance /srv/board/instance.toml start --foreground
```

A process manager can own the foreground command. Ordinary clients use `gb` with
an endpoint or connection file; they do not need the deployment filesystem or
PostgreSQL credentials. Local startup and offline database maintenance are separate
from API management. See [CLI guide](local-cli.md).

Set `externally_managed = true` for deployments where users must not accidentally
start services/workers locally. The deployment owner explicitly supplies
`--allow-local-processes` inside the intended allocation. This override does not
enable later dashboard launches. It is also available for intentional manual use.

## Access

The service exposes `/api/...` and optional static dashboard files on one port.
Configure `allowed_origins` for browser access, including SSH-forwarded origins.
Set `[server.auth]` with `admin_password_hash`, `session_secret`, and optional
`session_version`; `gammaboard auth hash-password --password-stdin` generates the
hash. Configuration and credential files should remain private.

The dashboard uses a session cookie. CLI clients can obtain a bearer token with
`gb auth login --password-stdin --output TOKEN_FILE`. Session version/secret changes
invalidate existing tokens. Use HTTPS outside a trusted network; a site reverse
proxy may provide TLS, but is not required for local operation.

## HPC

The Slurm/Apptainer helper submits a control allocation and starts the foreground
service there. It manages external worker submissions through HTTP, including
leases, progress, reconciliation, and cleanup acknowledgment. The service knows
nothing about Slurm. Ordinary clients use the published API endpoint; worker jobs
receive their own frozen database configuration.

See [Slurm guide](../ops/slurm/README.md) and [launch protocol](cluster-launches.md).
Normal shutdown goes through the API; forced scheduler teardown remains the
helper's responsibility when the service is unavailable or still queued.
