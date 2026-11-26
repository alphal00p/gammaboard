# Management API

The CLI, dashboard and deployment helpers send authenticated JSON requests to
`POST /api/v1/manage`. Each request has an `operation` discriminator. The complete
schema is [Request](../crates/gb/src/protocol.rs); unknown operations and fields
are rejected. Use matching client/service revisions.

The service owns lifecycle coordination and management. Workers use PostgreSQL
directly for computation. Connections, CLI commands and local/remote file ownership
are documented in the [CLI reference](local-cli.md).

## Authentication and endpoints

Authenticate with `Authorization: Bearer TOKEN` or the dashboard session cookie.
`POST /api/auth/login` accepts `{ "password": "..." }` and returns a token.
`POST /api/auth/session` checks browser access and makes browsers send their Origin,
including through same-origin proxies. Browser origins are configured explicitly;
see [deployment access](deployment.md#authentication-and-browser-access).

| Endpoint family | Purpose |
| --- | --- |
| `/api/v1/manage` | Run/task/worker/template management, status, logs and shutdown |
| `/api/v1/database/backup` | POST to stream a PostgreSQL custom-format backup |
| `/api/runs`, `/api/workers`, `/api/templates`, `/api/logs`, `/api/settings` | Dashboard reads and specialized panels |
| `/api/histogram-bundle/*` | Histogram import/export |
| `/api/node-launch-requests/*` | External-controller claims, leases and progress; see [launch protocol](cluster-launches.md) |

Offline probes and database administration use `gb local`; the management protocol
accepts no local-execution override. `status.database` includes reachability and
schema version; status also includes effective deployment configuration.

## Runs and retry receipts

```json
{"operation":"run_create","request_id":"experiment-007","toml":"name = 'experiment'","base_dir":"/shared/project","start":true,"max_evaluators":4}
```

`run_create` returns `request_id`, `created` (including `run_id`) and `start`.
Optional `export_path` registers an absolute deployment-side result path with a
separate export status. Creation remains committed if starting or export
registration fails. Those partial outcomes return successful HTTP responses;
inspect the receipt, not just the HTTP status. Failures before creation return a
non-success status and `{error}`.

A request ID is stored atomically with creation. Identical input replays the same
run; changed input or a deleted-run key is rejected. Names are not deduplication
keys. After an ambiguous transport failure, recover through the same creation key
or inspect state before retrying. Other mutations have no generic exactly-once
guarantee.

```json
{"operation":"run_start","run":"42","max_evaluators":4}
```

`run_start` returns `run_id` and `start`. `started` confirms admission of the complete
worker group; readiness is separate. `blocked` includes code `workers_unavailable`,
`retryable: true` and an availability summary. It holds no partial group and queues
no durable start. `gb run start --wait-for-workers` polls admission until its deadline.

Creation may return `not_requested`; unsuccessful execution may return `failed`.
The CLI additionally reports `timed_out` or `unknown` when waiting expires or an
RPC outcome is uncertain. It prints the receipt before exiting nonzero for an
unsuccessful requested step. Waiting never recreates the run.

## Tasks, exports and removal

`task` carries a nested `action`: `list`, `export`, `append`, `duplicate`, `edit`,
`tune`, `campaign_scheduling`, `campaign_tuning`, or `remove`. Edits accept
`expected_toml` for optimistic concurrency. Task action IDs accept integers or
decimal strings; browsers should use strings to preserve bigint precision.
Campaign actions carry a `settings` object. `run_results` with `task` inspects a task.

`run_export` requires `kind: "definition"` (returns `{toml}`) or `"results"`
(returns a versioned archive). `run_import_results` accepts a format-2 `bundle`
and optional `name`, creating a read-only run without evaluator execution.
Uploads are limited to 64 MiB. Archives contain terminal-task definitions and
latest snapshots, not restart state or complete history.

`run_remove` accepts `runs`, `all` and `dry_run`. It returns `{operations:[...]}`
immediately; poll each operation using:

```json
{"operation":"run_removal_status","operation_id":"..."}
```

Status is `running`, `completed` or `failed`, with an error on failure. Deletion
survives a disconnected caller. Running duplicates share an operation; parent and
child selection deletes their tree once. Operations remain in memory for at least
an hour after completion. After expiry or service restart, reconcile the run list
before retrying. Submission success is distinct from deletion completion.

## Workers

```json
{"operation":"worker_launch","source":{"kind":"new","count":6}}
```

| `source.kind` | Fields and behavior |
| --- | --- |
| `new` | `count` or grouped `toml`; optional `name_prefix`, `max_start_failures`, `args` |
| `saved` | `name`, optional `expected_uuid`, `restore_only`; use a stored launch recipe |
| `restore_saved` | Request the fleet marked for restoration |

The instance selects the execution backend. Submission returns
`{request_ids, node_names}`; it does not imply registration. Inspect progress with
`worker_requests`. `worker_request_cancel` works before a controller claims the
request. Claimed jobs require worker/deployment controls. `restore_only` requires
saved restoration intent still to be present.

`worker_list` accepts `state` (`live`, `offline`, `all`), optional exact `name` and
optional `run`. `worker_inspect` returns one record in the same schema.
`GET /api/workers?state=...&run_id=...` supplies that model for dashboard polling.
Saved launches, unassignment and restoration cancellation accept `expected_uuid`
to reject stale worker identities.

## Shutdown

`stop` accepts `wait` and optional `timeout_ms`, returning a durable `stop_id`.
With `wait: true`, accepted runs finish before workers stop. The service completes
exports, drains/checkpoints workers and waits for external cleanup acknowledgment.
Poll `status` for the matching ID and inspect `stop_result.success`.

The service publishes `ready_to_exit` briefly before shutting down. A lost
connection before confirmation is unconfirmed shutdown. Deployment owners verify
actual process/allocation termination and acknowledge external cleanup through
`acknowledge_cleanup`. See [durable shutdown](cluster-launches.md#durable-instance-shutdown).

CLI HTTP requests have bounded response timeouts, including each polling request.
Wait deadlines bound the whole wait; a timeout does not cancel accepted launch,
deletion, export or shutdown work.

## Campaign child edits

Child editing uses `operation: "task"` with `campaign_children_preview` and
`campaign_children_apply` actions. Both include the campaign task `id` and an
`edit` object, using the same bigint ID handling and authentication as other task
mutations. For example:

```json
{"operation":"task","run":"42","action":{"action":"campaign_children_preview","id":"57","edit":{"action":"remove_task","task_name":"extra","children":[43,44]}}}
```

The preview lists applicability and returns an opaque revision. Submit the same
edit with `expected` set to the preview and a stable `request_id` to
`campaign_children_apply`. Retries reuse that ID. Changed definitions or
eligibility require a fresh preview. Archived campaigns and adoption of archived
runs are rejected. These operations participate in instance admission/draining;
they do not use the retired per-task mutation routes.
