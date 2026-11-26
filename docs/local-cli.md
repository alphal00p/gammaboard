# CLI reference

Use `gb COMMAND --help` for all arguments and defaults. In a checkout, `./gb`
builds and runs the client; installed binaries are invoked as `gb`.

| Interface | Role |
| --- | --- |
| `gb` | Lightweight client for the instance's HTTP API |
| `gammaboard` | Service, worker runtime and offline utilities |
| `gb local` | Commands within `gb` that invoke the local `gammaboard` executable |

In a checkout, the `./gb` wrapper also prepares frontend dependencies and assets
for `start`/`restart` when needed; `--api-only` and `start db` skip frontend work.
`gb start` and `gb restart` invoke the local runtime executable. Ordinary management
commands, including `gb stop`, act through the API. Workers communicate directly
with PostgreSQL. See [setup](quickstart.md) for a first run and
[deployment](deployment.md) for installation, instance settings and browser access.

## Choose an instance

`--instance FILE` selects a deployment file or a minimal client connection file.
The default is `./gammaboard.toml`. Local operations require a full deployment file;
remote management needs only an API endpoint and credentials.

```toml
[client]
endpoint = "https://board.example.org"
token_file = "session.token"
```

Paths in these files resolve relative to the file. `--endpoint URL` and
`--token-file FILE` override connection settings; their environment equivalents
are `GAMMABOARD_ENDPOINT` and `GAMMABOARD_TOKEN_FILE`.

```bash
gb --instance ./client.toml config show
gb --instance ./client.toml config validate
gb --endpoint https://board.example.org auth login --password-stdin --output session.token
```

`gb config reload --origins` asks the running service to reread its deployment
file's `server.allowed_origins` and `server.allow_loopback_origins`. It applies only
browser-origin settings without restarting; `gb status` shows the effective
configuration. Loopback HTTP(S) origins on any port are trusted by default; shared
deployments can set `allow_loopback_origins = false` and use exact origins.
See [browser access](deployment.md#authentication-and-browser-access).

Login reads the password from stdin and writes a private token file without
replacing an existing file. It does not require an existing token.

## Start and stop

```bash
gb start --workers 6 --wait
gb start db
gb start server
gb start --api-only
gb start --foreground
gb restart
gb stop
gb stop --wait
```

- `start` provisions a managed database and starts the control/API service. It
  serves dashboard assets when available. `--api-only` disables static serving.
  Startup prints a clickable `Dashboard: http://…/` link when the frontend is
  served. JSON output includes `dashboard_url` (`null` when unavailable) and `api_url`.
- `start db` starts only PostgreSQL. `start server` uses the configured running
  database. `--foreground` keeps the service attached for a process manager.
- `--workers N` submits a request for **N additional workers** using the instance's
  launch policy. `--wait` waits for their registration. Startup alone launches none.
- `--restore-workers` requests restoration of workers saved by graceful shutdown.
  Repeating `start` reuses a healthy instance with matching configuration.
- `restart` stops and starts the whole local instance.
- `stop` checkpoints and stops workers, completes exports and coordinates deployment
  cleanup. `stop --wait` first waits for accepted runs to finish. New work is
  rejected during draining. `--timeout DURATION` bounds waiting.

Shutdown errors retain the service for inspection. Application completion and
process/allocation termination are separate: the local owner or Slurm helper
confirms termination. An API disconnect does not confirm successful shutdown.

For an externally managed deployment, local startup and computation require
`--allow-local-processes` on the intended host/allocation. That override is local
and does not change the service's worker-launch policy. See [HPC deployment](deployment.md#hpc).

## Validate configuration and runs

| Command | Where and what it validates |
| --- | --- |
| `gb config validate` | Client connection settings on this host |
| `gb local config validate` | Full deployment configuration on this host |
| `gb run validate FILE` | Run preflight on the service |
| `gb local run validate FILE` | Run preflight on this host |
| `gb local run validate FILE --probe` | Local preflight plus task-runtime construction |

Run preflight can execute evaluator programs for descriptor/domain discovery.
Local run validation uses the deployment's process gate. `gb status` shows the
running service's effective configuration.

## Runs and tasks

Run references accept an ID or an unambiguous name. Use IDs when names repeat.

```bash
gb run list
gb run inspect RUN
gb run create ./run.toml --start --max-evaluators 5
gb run start RUN --timeout 2m
gb run wait RUN --until ready --timeout 2m
gb run wait RUN --timeout 10m
gb run pause RUN
gb run results RUN
gb run results RUN --task TASK_ID
gb run duplicate RUN NEW_NAME
gb run remove RUN --dry-run
gb run remove RUN --yes
```

For a standalone integration, six workers allow one sampler/aggregator and five
evaluators. `--max-evaluators` is a ceiling; omit it to use compatible free capacity. Starting admits a complete compatible worker group;
`run start` and `run create --start` automatically retry blocked admission for up to
**10 seconds**, allowing workers from a completed run to finish releasing their
assignments. Use `--timeout 0` for one immediate attempt or `--timeout 2m` for a
longer wait. This timeout controls worker admission, not run completion; it never
leaves a background queued start. Only unavailable compatible capacity is retried.
With `run create`, an explicit `--timeout` requires `--start`.

`run wait RUN` defaults to successful completion and has its own **60-second**
timeout; use `--timeout 5m` for longer computations. Timing out does not stop the run.

| `--until` | Condition |
| --- | --- |
| `completed` (default) | All tasks completed successfully; worker release can still be in progress. |
| `ready` | An active task, one sampler/aggregator, exactly `--evaluators` active evaluators (default 1), and fresh performance reports. |
| `idle` | No live workers actively assigned or instructed to execute this run; this also includes paused, failed, and never-started runs. |

`--max-age` (default `10s`) controls report freshness for `ready`. Failed tasks make
`ready` and `completed` fail; `idle` does not assert successful completion.

Creation can succeed while starting fails. The receipt retains `created.run_id`,
and the CLI exits nonzero after printing the unsuccessful start status (`blocked`
for an immediate attempt, `timed_out` after exhausting the worker wait). Retry that
run with `run start ID`. To recover safely after losing the entire creation response,
reuse an explicit `--request-id` with identical input:

```bash
gb --json run create ./run.toml --request-id experiment-007 \
  --start --timeout 2m
```

Names do not deduplicate creation. See [API receipts](management-api.md#runs-and-retry-receipts)
for structured statuses and retry rules.

```bash
gb run task list RUN
gb run task append RUN tasks.toml
gb run task edit RUN TASK_ID task.toml
gb run task tune RUN TASK_ID queue-tuning.toml
gb run task duplicate RUN TASK_ID
gb run task export RUN TASK_ID --output task.toml
gb run task remove RUN TASK_ID --yes
```

Definition edits/removal apply to eligible pending tasks. Tuning changes queue
settings. Duplication creates fresh work without checkpoints or results. Run removal
drains and deletes the run tree; the CLI polls the background operation to completion.
`run pause` and `run remove` also accept multiple references or `--all`.

## Files and exports

Input files are read on the caller and uploaded. Referenced executables, datasets
and child-run files must exist on the deployment. `--base-dir /shared/project` on
`run create` or `run validate` sets the directory for relative references; the
default is the service's working directory.

```bash
gb run export RUN --kind definition --output definition.toml
gb run export RUN --kind results --output results.json
gb run create ./run.toml --start --export-path /shared/results/result.json
```

`--output` writes atomically on the caller without overwriting. Definition exports
are TOML; result exports are JSON inspection archives. `--export-path` is an absolute
path on the deployment: the service writes it after completion, even if the client
exits or the service restarts. Export failures appear in `gb status`.

## Workers and templates

```bash
gb worker launch --count 6 --wait --timeout 2m
gb worker launch --config workers.toml
gb worker list --state all
gb worker inspect NAME
gb worker assign NAME RUN --role evaluator
gb worker unassign NAME
gb worker stop NAME
gb worker stop --idle
gb worker stop --all
gb worker start NAME --wait
gb worker restore --wait
gb worker cancel-restoration NAME
gb worker requests list
gb worker requests inspect REQUEST_ID
gb worker requests wait REQUEST_ID
gb worker requests cancel REQUEST_ID
```

All launches submit durable requests; the instance chooses local or external
execution. `launch` creates workers, `start NAME` uses a saved recipe, and `restore`
starts only workers marked for restoration. Each returns `request_ids` and
`node_names`. Waiting times out without canceling accepted requests.

Worker lists use one schema for `live` (the default), `offline`, or `all`.
Inspection returns one worker. Canceling restoration clears saved intent; canceling
a launch request is allowed only before a controller claims it. Submitted jobs
require worker/deployment controls. See [worker behavior](operations.md#manage-work).

```bash
gb template list run
gb template show run tetrahedron-native
gb template save run NAME definition.toml
gb template remove run NAME --yes
```

Template kinds are `run`, `task`, and `worker`. Saving an existing name requires
`--yes` to overwrite it.

## Database maintenance

`gb status` includes database health and schema version. `gb db backup --output
FILE` downloads a backup through the API. Local commands use the deployment's
configured database directly:

| Command | Effect |
| --- | --- |
| `gb local db status` | Check database availability without the API |
| `gb local db backup --output FILE` | Write a PostgreSQL backup on this host |
| `gb local db create` | Create and migrate the configured logical database |
| `gb local db migrate` | Apply supported schema migrations |
| `gb local db restore FILE --yes` | Replace database contents from a backup |
| `gb local db reset --yes` | Recreate the logical database from migrations |
| `gb local db delete --yes` | Delete the logical database |
| `gb local db purge --yes` | Remove the entire managed PostgreSQL cluster |
| `gb local db stop` | Stop managed PostgreSQL after the service has stopped |

Stop the service before create/migrate/restore/reset/delete/purge. External clusters
cannot be purged. Backups exclude external artifacts; see
[backup and recovery](operations.md#backup-and-recovery).

## Diagnostics and automation

```bash
gb status --json
gb logs --service server
gb logs --service db
gb logs --run RUN --follow --jsonl
gb run performance RUN --duration 10s --json
gb build-info
gb build-info --server
gb completion bash
```

`--json` emits one document for finite commands. Streaming logs use `--jsonl`.
`build-info` inspects the client offline; `--server` queries the service.
Deployment owners use `gb control acknowledge-cleanup REQUEST_ID` only after
confirming external jobs are gone; see the [shutdown contract](cluster-launches.md#durable-instance-shutdown).

## Inspecting retained or imported results

Benchmark helpers accept `--keep-instance /absolute/path`. They stop normally and
write `retained-instance.json` with the command to reopen each retained instance
without launching workers. See [benchmarking](benchmarking.md).

```bash
gb run export RUN --kind results --output results.json
gb run import-results results.json --name "Archived calculation"
```

Imports create read-only dashboard archives from terminal tasks, without executing
evaluators. They preserve task definitions, latest snapshots, plots, histograms and
provenance. Full execution/performance history, child runs and resumable state are
not included. Use a database backup for recovery. Remove archives with `run remove`.
