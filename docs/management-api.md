# Management API

GammaBoard hosts lifecycle coordination, the HTTP API, and optional dashboard
assets in one process. Workers continue to communicate directly with PostgreSQL.
The standalone `gb` executable depends only on the client/protocol crate, not the
computation library or PostgreSQL driver.

## Version 1

Authenticated clients post a JSON request to `/api/v1/manage`. Each request has an
`operation` discriminator and the fields defined by `Request` in
`crates/gb/src/protocol.rs`. Unknown operations/fields are rejected. Responses are
JSON values; unsuccessful operations return non-success HTTP status and an error.
The existing dashboard and launch-controller domain routes remain available and
use the same application functions.

Examples:

```json
{"operation":"run_list","name":null}
```

```json
{"operation":"run_create","toml":"...","base_dir":null,"name":null,"start":true,"max_evaluators":4,"export_path":"/shared/project/result.json"}
```

```json
{"operation":"worker_launch","count":6,"toml":null}
```

The protocol covers runs, tasks, workers, launch-request inspection, templates,
logs, database status, and instance shutdown. Effective deployment configuration
is part of status; client configuration is inspected and validated locally. Request
and result shapes are independent of presentation in the CLI. `gb --json` emits
one JSON document for finite commands. Log streaming uses `--jsonl`.

## Connections and credentials

Clients can use `--endpoint`, `GAMMABOARD_ENDPOINT`, or a connection file:

```toml
[client]
endpoint = "https://gammaboard.example.org"
token_file = "session.token"
```

Select the file with `--instance FILE`. Token paths resolve relative to that file.
A local deployment's instance file also supplies connection information through
its server address. A remote client does not need database credentials or access
to the service's control directory.

`gb auth login --password-stdin --output session.token` obtains a session token
without putting a password on the command line. Token files are created privately
and existing files are not overwritten. Tokens use the existing session expiry
and revocation rules. HTTP requests may authenticate with `Authorization: Bearer`
or the dashboard session cookie. Use HTTPS for connections outside a trusted
network; a reverse proxy can provide TLS without being part of the application.

## File ownership

`gb run create ./run.toml` uploads the local document. Referenced resources must
exist on the deployment. `--base-dir /shared/project` explicitly supplies the
server-side directory used to resolve references; without it, references resolve
from the service's working directory.

`gb run results RUN --output ./result.json` downloads a result to the caller.
`gb run create ... --export-path /shared/project/result.json` registers an export
owned by the service, which continues after the caller exits. Export paths must
be absolute. Local output files are written atomically without overwriting.

`gb db backup --output ./database.dump` streams a completed PostgreSQL custom-format
backup from `POST /api/v1/database/backup`. Offline restore, migration, reset,
delete, and cluster purge are deployment operations executed by the separately
installed service executable, not arbitrary remote file/process operations.

## Long-running work and shutdown

Launch requests and exports remain durable database records. Launch submission
never runs a worker in the HTTP handler. The service chooses the backend, and
local/external controllers consume the persisted requests. Launch, start, and
restore return `request_ids` and `node_names`; `--wait` explicitly waits for request
fulfillment. Request cancellation is allowed only before a controller claims it;
otherwise it fails with guidance to use worker or deployment controls. `gb run wait` also polls without holding a server request open. Mutations
are sent once: after an ambiguous transport failure inspect state before retrying;
this API does not promise generic exactly-once execution.

Shutdown returns a durable `stop_id`. The client polls status for that exact
request. The service drains/checkpoints work, completes exports, and waits for
external cleanup acknowledgment before publishing `ready_to_exit`. A short grace
period lets clients observe this result before API and database shutdown. A lost
connection before acknowledgment is reported as unconfirmed, never success.
Local process exit and Slurm allocation termination are verified by their owners.

All HTTP requests have bounded response timeouts. Explicit wait deadlines also
bound each HTTP poll, so a stalled response cannot indefinitely extend a CLI
wait. Measurement requests allow their explicitly requested duration plus a
bounded response margin. Local probes and offline database maintenance belong
to `gb local`; the management protocol accepts no local-execution override.
