# External worker launch contract

External launchers use authenticated API calls under `/api`. The server accepts
the current grouped node-launch TOML, validates/normalizes resources and atomically
reserves each worker name and connection capacity. There is no flat-argument
fallback. The ordinary local launcher uses the same canonical resource recipe.

## Lease and submission API

| Endpoint | Request | Result |
| --- | --- | --- |
| `POST /node-launch-requests` | `{ "toml": "..." }` | Reserved grouped request |
| `POST /node-launch-requests/claim-external` | `{ "launcher_id": "unique-owner", "request_id": 123 }` (ID optional) | `{ "request": null }` or leased request |
| `POST /node-launch-requests/123/lease` | `{ "lease_token": "...", "release": false }` | Extend lease; `true` expires it immediately |
| `POST /node-launch-requests/123/progress` | `{ "lease_token": "...", "node_name": "w-1", "state": "submitted", "job_id": "456" }` | Durable per-worker transition; optional `error` |

A claimed request contains string `id`, `args.groups`, `lease_token`,
`lease_seconds=120`, and `workers`. Each worker has `node_name`, stable
`submission_key`, `state`, nullable `job_id`/`error`, and `registered`. Groups
include canonical `config`, `capabilities`, reserved `node_names`, `count` and
`max_start_failures`. Claiming excludes another launcher's live lease and uses
`SKIP LOCKED`; expired requests are recoverable. Renew every 20 seconds. A stale
or expired token cannot write progress, even after a replacement launcher starts.

| Worker state | Meaning and allowed next step |
| --- | --- |
| `pending` | Name/capacity reserved; save `submitting` before contacting Slurm |
| `submitting` | Durable intent; receipt may be lost; adopt a matching job as `submitted` |
| `submitted` | Immutable numeric Slurm `job_id` recorded; await registration |
| `connected` | Matching node registered; authoritative successful launch |
| `failed`, `canceled` | Terminal unsuccessful launch; unregistered capacity is released |

An unfinished worker may become connected directly from registration, or fail/be
canceled. Successful registration is never downgraded by a late scheduler failure.
A late submission receipt can fill in the job ID after registration. Failed workers
are not retried automatically. The one explicit reset is `submitting -> pending`
with no job ID and error `operator confirmed no submission`; the operator must
first establish that no scheduler allocation exists.

Claims, intent and receipts use synchronous database commits. This protects
submission records even when sampling storage uses asynchronous commits. No
transaction spans `sbatch` and PostgreSQL: after a crash, match the deployment and
submission identities in Slurm names/comments, consult accounting, and preserve
uncertainty when a unique allocation cannot be established. A missing job is not
a terminal job. Scheduler errors must not trigger worker termination or resubmission.

Request `started_count` counts recorded jobs or connected workers. Each failed
worker releases its reservation while siblings may still be starting. A request
finishes only after all workers are connected/failed/canceled; any failed/canceled
worker makes it failed. Registration also reconciles success independently of the
watcher. Connected means launched, not that the worker stays alive indefinitely.

## Deployment and build identity

The UBELIX helper scopes a deployment to canonical workspace plus port offset and
stores a unique generation ID, control job, image digest and config snapshots.
The active pointer and submissions use a workspace filesystem lock; shutdown and
submission cannot interleave. Service images have fixed paths and can only be
rebuilt while every deployment in that workspace is stopped. Workers verify the
recorded image digest. Queued jobs retain their launcher and configuration even
when the checkout changes. `/health` reports `deployment_id` and `build_id`; helpers
verify both before authenticated control operations.

`gammaboard build-info` reports the package version, source revision, compiled
features, build ID and database model. When `GAMMABOARD_DEPLOYMENT_ID` is set, server
startup binds the database to that deployment/build. Node registration and lease
renewal are guarded inside PostgreSQL, including writes from old binaries that
omit identity fields. Binding and registration serialize through an advisory lock;
a replacement refuses live workers and cancels unfinished requests from the old
cohort. A bound database cannot be served without a deployment ID. Unbound local
deployments do not enforce this cohort restriction.

Build identity covers the application revision/features. Image SHA-256 separately
pins its container dependencies. Neither is authentication against a trusted
operator who can edit the database or deployment files. See the
[UBELIX operator guide](../ops/ubelix/README.md) for publication, shutdown and repairs.
