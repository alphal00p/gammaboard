# UBELIX deployment

Run sync on your workstation and `python3 ubelix.py` on a UBELIX login node.
The helper requires Python 3.11+, Slurm (`sbatch`, `squeue`, `sacct`, `scancel`),
and Apptainer. A workspace is the canonical directory containing the installed
helper. A deployment is one control allocation, its workers, and an immutable
image/configuration snapshot. Use one workspace and port offset consistently.

## Managed files and private configuration

```bash
just sync-ubelix
# Or another relative folder below /storage/research/itp_localunitaritydata:
just sync-ubelix ubelix "$USER/gammaboard"
```

Sync mirrors only `ops/`, `gbops/`, `docs/`, and `resources/templates/`, and replaces
`ubelix.py` and this README. Deleted repository files disappear from those trees.
Templates combine the shared run/task cards with UBELIX node cards. **Do not keep
private edits in managed trees.** Move them out before syncing this revision.

Private configuration belongs in `config/server.toml` and `config/runtime.toml`.
These merge over the managed server defaults and embedded runtime defaults;
`up --server-config PATH --runtime-config PATH` selects other private overrides.
The helper owns base ports, database URL, and local/external launch mode. Use
`--port-offset`, rather than overriding ports. Configuration must use TOML 1.0
syntax supported by Python's `tomllib`; run/task cards support TOML 1.1.

Sync preserves `config/`, `deployments/`, `images/`, `artifacts/`, logs, databases,
physics states, and process environments. Shell environment overrides belong in
`~/.config/gammaboard/slurm.env`, sourced by the batch scripts. Secrets in server
configuration are copied into private deployment snapshots; protect/backup those
snapshots accordingly. Environment overrides are still read at job start.

## Build and select images

```bash
python3 ubelix.py build gammaboard --revision <FULL_COMMIT_SHA>
python3 ubelix.py build gammaloop --revision <FULL_COMMIT_SHA>
python3 ubelix.py build apptainer resources/runtimes/madnis/madnis.sif resources/runtimes/madnis/apptainer.def
```

Service builds resolve the revision from the remote repository. Local uncommitted
or unpushed changes are not included. Each build has private sources, Cargo target,
staging, and temporary image paths under `artifacts/builds/`. Only dependency
caches are shared. Rust, Node, and Debian base images are pinned by digest in
`ops/build/images.lock.json`; Rust builds use `--locked` and a generic x86-64 CPU
target. The builder and runtime share the Bookworm ABI. Package versions and
compiler information are retained in the image; apt repositories can still change,
so this is not a promise of byte-for-byte reproducible builds.

Image definition tests run during building. GammaBoard publication additionally
requires starting its bundled PostgreSQL, API, frontend and two workers, then
completing a 32-sample run inside the image. GammaLoop requires a successful CLI
startup. Logs and smoke evidence remain in the unique build directory. Failed
builds leave the previously published image selected.

Successful images are published at
`images/<family>/<source-sha>/<image-sha256>/<family>.sif`, alongside `image.json`.
`images/<family>/current.json` selects the default for **new** deployments. Active
workers/control retain their selected digest even after another build completes.
Use `up --image PATH/TO/image.json` to select a particular published image.
Generic runtime builds refuse to overwrite an existing output; choose a new path.

## Start and reuse

```bash
python3 ubelix.py up --watch
python3 ubelix.py up --single-node --port-offset 10 --local-port 9000
```

Normal mode uses one control job for PostgreSQL/API/nginx and separate worker
allocations. Single-node mode launches workers inside the control allocation.
Ports are frontend/API/PostgreSQL `8080/4000/5400` plus the offset. Managed database
paths are also isolated by offset. A custom `--local-port` is included in allowed
browser origins. `up` waits for a matching deployment/build health response before
printing the SSH tunnel command; run the tunnel locally and open its local port.

Useful options are `--time 00:45:00`, `--startup-timeout 180`, `--copy` (terminal
clipboard escape), and `--resume-workers`. Resume requests previously configured
workers; without it, startup suspends them. `--watch` keeps reconciling dashboard
requests while the helper runs. Otherwise run `watch-requests` separately.
`--time` and `--resume-workers` apply when creating a control job; they do not
reconfigure an already running control allocation.

The active pointer is scoped by canonical workspace and port offset. Manifests
under `deployments/<id>/` record the control job, image digest, origins, private
configuration snapshots and launch recipes. Queued jobs also retain a copy of
the launcher, so a managed sync cannot remove their scripts. Job names and Slurm
comments contain the deployment ID and submission identity.

Repeating `up` reuses a matching active deployment. Different mode, configuration,
image or tunnel origin requires stopping it first. An unavailable scheduler or
missing accounting record is **unknown**, not proof that a job ended; the helper
refuses to create a duplicate control job. A startup timeout leaves the deployment
recorded and can be followed by `status`, `up` or `down`.

## Worker requests and recovery

```bash
python3 ubelix.py submit-workers --count 2 --prefix w
python3 ubelix.py watch-requests
python3 ubelix.py watch-requests --once
```

Pass the selected `--port-offset` to each command. Dashboard launch cards use only
the current grouped schema:

```toml
[[groups]]
count = 1
name_prefix = "gpu"
max_start_failures = 6
config = { gpus = "h100:1", cpus_per_task = 4 }
```

The server normalizes resources once, reserves names, and supplies canonical
configuration/capabilities to the launcher. `gpu = "h100:1"` becomes
`gres = "gpu:h100:1"`; typed `gpus` and GPU `gres` also register `gpu=1` and enable
Apptainer `--nv`. Use exactly one GPU allocation spelling. Numeric/bool `gpu`
shorthand chooses `rtx4090`; use typed forms for another model. CPU aliases `cores`,
`nr_cores`, `cpus`, and `cpus-per-task` become `cpus_per_task` and capability `cpus`.
Conflicting CPU aliases, GPU declarations, and `mem` with `mem_per_cpu` are rejected.
Supported Slurm options are `account`, `partition`, `qos`, `wckey`, `reservation`,
`gres`, `gpus`, `cpus_per_task`, `mem`, `mem_per_cpu`, `time`, `constraint`, `nodelist`,
and `exclude`. Additional numeric keys are custom worker capabilities.

Each request has a renewable 120-second database lease and a per-worker submission
record. The launcher saves intent before `sbatch`, then immediately saves its job
ID. Restarting a watcher adopts matching scheduler/accounting jobs. A worker's
registration is authoritative even if the job-ID response was lost. Confirmed
terminal jobs that never registered fail their individual reservation. Temporary
scheduler failures do not kill healthy workers or cause automatic resubmission.

There is no atomic transaction spanning PostgreSQL and Slurm. If intent was saved
but no unambiguous job can be found, the helper reports uncertainty and leaves it
for explicit resolution. Stop the watcher and allow its lease to expire, inspect
Slurm/accounting, then either adopt the identified job or confirm no job was sent:

```bash
python3 ubelix.py resolve-submission 123 worker-1 --job-id 456789
python3 ubelix.py resolve-submission 123 worker-1 --not-submitted
# The corresponding repair for an interrupted control submission:
python3 ubelix.py resolve-control --job-id 456790
python3 ubelix.py resolve-control --not-submitted
```

The repair commands check job ownership and refuse an absence claim if matching
jobs are visible. `--not-submitted` is an operator assertion: accounting gaps are
not sufficient evidence. It enables a subsequent retry; it does not submit a job.
The database/API state machine is specified in [cluster launches](docs/cluster-launches.md).

## Stop and inspect

```bash
python3 ubelix.py status --port-offset 10
python3 ubelix.py down --port-offset 10
```

`down` cancels a pending control allocation directly. For a running control it
suspends workers through the API, waits up to `--worker-timeout` seconds, cancels
remaining owned workers, then cancels the control. A finished control still gets
its stranded workers cleaned up. Watchers cannot submit while shutdown runs.
Only jobs belonging to this deployment are canceled. Failed queries or unknown
control status remain errors; retry after inspecting scheduler availability.

The helper never deletes database directories. Use GammaBoard's `db backup` and
`db restore` for maintenance; see [operations](docs/operations.md). This revision
requires a fresh model-3 database, not an in-place upgrade from the prior schema.
The database rejects worker registrations and lease renewals with a different
deployment/build identity and refuses cohort replacement while workers are live.

## Access and layout

Protected commands take `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD` when
authentication is enabled. Defaults are passwordless. Distributed mode enables
`--postgres-trusted-network`; every user/device reaching the database is trusted.
A matching build check is not an authorization boundary for database administrators.

```text
<workspace>/
  ubelix.py, gbops/, README.md
  ops/{build,config,slurm}/       # managed
  docs/, resources/templates/  # managed
  config/                      # private overrides
  deployments/<id>/            # manifest, configs, launcher and worker recipes
  images/<family>/             # immutable images and current.json
  artifacts/{builds,download-cache}/
  logs/slurm/
  resources/{db,states,runtimes}/
```

Start with `resources/templates/runs/installation-smoke.toml`. Scientific runs
need their matching physics states and process runtimes; see
[examples](docs/quickstart.md#examples) and [deployment](docs/deployment.md).
