# CLI and local instances

`gb` is a lightweight HTTP client. `gammaboard` is the service/worker executable.
The control service owns lifecycle coordination, exports, local worker processes,
and the management API. It optionally serves dashboard assets on that same port.
Workers retain direct PostgreSQL access.

## Build and start

```bash
cargo build --release --workspace --bins
./gb start --workers 6 --wait
./gb run create resources/templates/runs/tetrahedron-native.toml \
  --start --max-evaluators 4 --export-path "$PWD/tetrahedron.json"
./gb stop --wait
```

The repository launchers build their respective packages. `./gb` builds only the
client for ordinary management commands. Starting locally invokes the separately
installed service executable (a sibling `gammaboard`, or
`GAMMABOARD_SERVICE_BINARY`). No frontend build is required for service startup.
To build the optional dashboard, run `npm ci` and `npm run build` in `dashboard/`.

`start --workers 6` submits a request for six **additional** workers, using the instance’s launch policy. `--wait` also waits for registration. There is no implicit
configured worker count. `start --restore-workers` submits requests for saved workers using the current instance policy. Repeating `start` reuses a healthy matching instance.

```bash
./gb start db                      # PostgreSQL only
./gb start server                  # Control/API against the configured DB
./gb start --api-only              # Control/API without static dashboard files
./gb start --foreground            # Service remains attached; suitable for Slurm
./gb restart                      # Ordered stop and local startup
```

`start db` cannot launch workers or own a foreground service. Normal management
commands require the API. There is no separate `stop server` operation: stopping
the control service stops management access. Missing dashboard assets do not
prevent the API from starting.

## Explicit configuration

Start from `ops/local/instance.example.toml`. Select a deployment file with
`--instance FILE`; the default is `./gammaboard.toml`. There is no home-directory
instance registry. Relative configuration paths resolve against that file.

- `name` labels the instance; its file and explicit paths identify the deployment.
- `externally_managed = true` blocks local bootstrap and computation unless the
  invocation explicitly supplies `--allow-local-processes`.
- `[paths]` declares state, logs, temporary files, templates, and resource roots.
- `[database]` selects managed PostgreSQL or an external database URL.
- `[server]` declares `host`, `api_port` (also accepted as `port`), optional
  `dashboard_dir`, allowed browser origins, authentication, and local-launch policy.
- `[logging]` and `[shutdown]` configure tracing and bounded checkpoint/worker stop.

Each concurrently running instance needs its own API port, managed database port,
data directory, state directory, logs, and temporary directory. File locks and a
database advisory lock prevent competing service owners.

Validate a deployment offline with:

```bash
./gb --instance /path/instance.toml local check-config
```

For a remote client, use `--endpoint URL` or a minimal connection file:

```toml
[client]
endpoint = "https://board.example.org"
token_file = "session.token"
```

`gb auth login --password-stdin --output session.token` obtains a machine session.
The client needs no PostgreSQL credentials, shared control directory, or native
computation dependencies. See [management API](management-api.md) for the contract.

## Runs, tasks, and files

```bash
gb run list
gb run inspect RUN
gb run validate ./run.toml
gb run create ./run.toml --start --max-evaluators 4
gb run wait RUN --until completed --timeout 2m
gb run results RUN --output ./result.json
gb run export RUN --output ./definition.toml
gb run pause RUN
gb run start RUN
gb run duplicate RUN NEW_NAME
gb run remove RUN --dry-run
gb run remove RUN --yes
```

Input documents are read on the caller and uploaded. Referenced executables,
datasets, and child-run resources must exist on the deployment; `--base-dir`
supplies an explicit deployment directory for relative run references.

`--output` on `results` or `export` writes locally, atomically without overwriting.
`run create --export-path /shared/result.json` schedules a server-side export that
survives client disconnection and service restart. It requires an absolute path.
Existing conflicting files remain untouched and the error is visible in `status`.

Task commands are `list`, `inspect`, `append`, `duplicate`, `edit`, `tune`, `export`,
and `remove`. Edits/removal retain the existing task-state restrictions. Duplication
copies definitions, not checkpoints; external resource paths remain unchanged.

## Workers and templates

```bash
gb worker launch --count 2
gb worker launch --count 6 --wait --timeout 2m
gb worker requests wait REQUEST_ID
gb worker requests cancel REQUEST_ID
gb worker list
gb worker list --offline
gb worker inspect NAME
gb worker assign NAME RUN --role evaluator
gb worker unassign NAME
gb worker stop --idle
gb worker stop --all
gb worker start NAME
gb worker restore
gb worker cancel-restoration NAME
gb worker launch-request list
gb template list run
gb template show run tetrahedron-native
gb template save run NAME ./definition.toml
gb template remove run NAME --yes
```

All launches are durable requests. The instance chooses the execution backend;
local fulfillment runs in a service-owned controller, and external fulfillment
runs in the helper. `launch`, `start NAME`, and `restore` return request IDs by
default. Add `--wait --timeout 60s` to wait for those requests to be fulfilled.
Timeouts do not cancel accepted work. `worker requests list/inspect/wait/cancel`
operate on the returned IDs. Cancellation is atomic and supported before a
controller claims a request; claimed/submitted jobs require worker/deployment
controls, and cancellation never pretends to have stopped them.

`--allow-local-processes` only authorizes local bootstrap, offline administration,
or direct worker execution on the current host. It is not sent to the API and
does not override the instance's launch policy. Worker execution itself is `gammaboard node run`, using the service
executable and its database/runtime configuration.

## Shutdown and recovery

`gb stop` checkpoints and stops workers. `gb stop --wait` first finishes the runs
accepted before draining, then completes exports and waits for external cleanup.
New work is fenced during draining. Timeouts and export failures retain the
service for inspection; an external-cleanup timeout retains the admission fence.

The API reports `ready_to_exit` once application shutdown is complete. The local
bootstrap owner or Slurm helper verifies actual process/allocation termination.
An API disconnect before completion is an error, never inferred success.

The service writes `state_dir/status.json` as an owner-side receipt, holds
`service.lock`, and records startup snapshots under `state_dir/deployments`.
Pending exports and shutdown requests live in PostgreSQL. Unexpected service
termination does not automatically kill worker computations or delete data.
Restart reacquires ownership and resumes durable coordination.

## Database maintenance and diagnostics

`gb db status` uses the API. `gb db backup --output ./database.dump` streams a
backup to the caller. `gb local db create`, `migrate`, `restore`, `reset`, `delete`, and
`purge` delegate to the local service executable and require local deployment
configuration. Restore/reset/delete/purge require `--yes`; restore/reset/delete
require the service to be stopped. `delete` removes the logical database; `purge`
removes the entire managed cluster. External clusters cannot be purged.

```bash
gb status --json
gb logs --service server
gb logs --service db
gb logs --run RUN --follow --jsonl
gb run performance RUN --duration 10s --json
gb completion bash
```

Service logs are `log_dir/service.log`; PostgreSQL logs are `log_dir/postgres.log`.
Finite JSON commands emit one document. Streaming logs use JSON Lines explicitly.

## Command scope

`gb config show` displays the resolved client endpoint and credential-file path;
`gb config validate` validates that connection configuration offline. It does not
claim to validate the running service. Use `gb local check-config` for full
deployment validation and `gb status` for the server's effective configuration.
Explicit endpoint/token-file overrides are applied before credentials are read.
`auth login` does not require an existing token file.

`gb build-info` reports the installed client without connecting; `--server`
requests service build information. `gammaboard` is exclusively the service,
worker, and offline utility executable; it does not forward management commands.

`gb db status/backup` are remote operations. Offline status, creation, migration,
restore, deletion, and cluster purge are explicitly `gb local db ...` operations.
They require a deployment file, never just a client connection file. `start` and
`restart` are convenient local bootstrap commands; restart always affects the
whole instance and accepts no `db`/`server` component argument.

`gb run validate FILE` validates through the API without executing a probe.
`gb local check-run FILE --probe` explicitly executes the probe on this host and
respects the deployment's process gate. Human output uses labeled summaries;
`--json` gives one machine-readable document. The unused `--quiet` flag is removed.
