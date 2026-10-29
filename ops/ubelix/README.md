# UBELIX deployment

Sync from your workstation; run `python3 ubelix.py` on a UBELIX login node with
Python 3.11+, Slurm and Apptainer. The canonical directory containing the installed
helper is its workspace. A deployment pins one image, configuration, Slurm profiles
and launcher scripts. Commands default to port offset zero.

## Clean redeployment

Stop any old foreground `up --watch` or `watch-requests` sessions and old build
jobs before upgrading the helper. These are replaced by a launcher supervised inside the control job.

On your workstation:

```bash
just sync-ubelix
# Or a separate workspace below /storage/research/itp_localunitaritydata:
just sync-ubelix ubelix "$USER/gammaboard"
```

On UBELIX, in that workspace:

```bash
python3 ubelix.py status
python3 ubelix.py down
# If the API is unavailable, cancel only jobs verified as this deployment's:
python3 ubelix.py down --force

# Delete the stopped deployment's database, socket directory and PostgreSQL log:
python3 ubelix.py reset-db --discard
# Omit --discard to archive those paths instead.

python3 ubelix.py build gammaboard --revision <FULL_PUSHED_COMMIT_SHA>
```

The build command prints its build ID and Slurm job ID. Wait for successful build
completion and inspect `logs/slurm/build/` and the build's smoke evidence. Publication
creates an immutable image but **does not change the selected image**. Select it
explicitly, using the build ID printed above:

```bash
python3 ubelix.py select-image artifacts/builds/<BUILD_ID>/published.json
python3 ubelix.py up
python3 ubelix.py submit-workers --count 6 --prefix cpu
```

`up` returns after the API and managed launcher are ready and prints the SSH tunnel
command. Run that tunnel on your workstation. No login-terminal watcher is needed.
Use `--port-offset N` consistently on `status`, `down`, `reset-db`, `up` and worker
commands when managing another instance. Skip `down`/`reset-db` for a new workspace.
If an installation predates deployment manifests, stop it with its original tools
and start in a new workspace; the current helper will not guess ownership of old jobs.

Latest GammaBoard requires a fresh database at an incompatible model change; see
[current format versions](docs/compatibility.md). `up` checks the selected image's
model against the managed database marker before submitting a job. An existing
unmarked database requires explicit reset. The binary's database compatibility
check remains authoritative. `up --fresh-db` archives an already stopped database
and starts fresh; it refuses to reset a running deployment. Reset is limited to
ordinary paths under `resources/db`; external or symlinked paths require manual
maintenance. Archives include the deployment/configuration metadata and pin their
matching image during pruning. Restore requires matching software and PostgreSQL.

After validating the replacement, inspect and apply artifact cleanup:

```bash
python3 ubelix.py prune
python3 ubelix.py prune --apply
# Optional: keep no additional retired deployment beyond current pointers:
python3 ubelix.py prune --apply --keep-deployments 0
```

Pruning removes confirmed retired deployment directories, unreferenced published
images and terminal build staging. It keeps current pointers, their images, active
or uncertain jobs, and one additional retired deployment per offset by default.
Private configuration, physics states, process runtimes, database archives and logs
are preserved. Untracked legacy build directories are left for manual inspection.
There is no whole-workspace deletion command.

## Private configuration and resources

Sync mirrors only `ops/`, `gbops/`, `docs/` and `resources/templates/`, and replaces
`ubelix.py` and this README. Deleted repository files disappear from those trees.
Keep private files outside them: sync preserves `config/`, `deployments/`, `images/`,
`artifacts/`, databases, physics states, process runtimes and logs.

Private files merge over managed defaults:

| File | Purpose |
| --- | --- |
| `config/server.toml` | Hosting, authentication and application shutdown settings |
| `config/runtime.toml` | PostgreSQL tuning, tracing and runtime settings |
| `config/cluster.toml` | Slurm `[control]`, `[worker]` and `[build]` resource profiles |

`up --server-config PATH --runtime-config PATH` selects alternative private files.
Ports, resource root, database URL and local/external launch mode belong to the
helper. Use `--port-offset`, not custom base ports/database URLs. Configuration is
TOML 1.0; run/task cards also support TOML 1.1.

For example, to change the control allocation without editing managed batch scripts:

```toml
# config/cluster.toml
[control]
cpus_per_task = 4
mem = "8G"
time = "02:00:00"

[build]
cpus_per_task = 8
mem = "24G"
```

Choose account, partition, QoS and time values permitted by your allocation.
Managed defaults use the existing `gratis`/`epyc2` profile: control is one hour,
workers thirty minutes and builds two hours. `up --time` overrides control walltime
for a new deployment. Profiles are frozen with the deployment/build. A running
allocation cannot be reconfigured by repeating `up`.

Control and worker submissions request a 180-second termination warning. Process
shutdown has a 120-second bound; application drain/stop settings must fit inside
that budget. When Slurm supplies its projected end time, new worker requests are capped to the remaining control lifetime,
leaving the shutdown margin; requests too near expiry remain pending. This uses
Slurm's [projected job end time](https://slurm.schedmd.com/sbatch.html#OPT_SLURM_JOB_END_TIME).
Worker walltime must exceed that shutdown margin. Queued workers can still start
late and stop when control ends.

Worker request groups override worker-profile resources. Declare GPU resources in
request groups so the server supplies canonical capabilities; GPU defaults in the
worker profile are rejected. CPU capability reflects the effective Slurm CPU count.
Single-node mode (`up --single-node`) uses the control allocation for local workers;
size that allocation accordingly. The worker Slurm profile applies to distributed
jobs. A GPU control profile enables NVIDIA bindings for single-node workloads.

If authentication is enabled, set `GAMMABOARD_ADMIN_PASSWORD` when creating the
deployment. The managed launcher needs it after your terminal exits. It is saved
in a private `launcher-auth.json` file (0600, inside a 0700 deployment directory),
not in the printed manifest. Protected helper commands reuse that credential unless
explicitly overridden. Treat deployment snapshots as secrets when backing them up.
Changing the launcher credential requires stopping and recreating the deployment.

`~/.config/gammaboard/slurm.env` remains available for site/module/environment setup
at job start. Resource settings belong in the profiles; changing this environment
file does not change pinned images, configs or submitted build recipes.

## Builds and image selection

```bash
python3 ubelix.py build gammaboard --revision <FULL_COMMIT_SHA>
python3 ubelix.py build gammaloop --revision <FULL_COMMIT_SHA>
python3 ubelix.py build apptainer resources/runtimes/custom-v2.sif path/to/runtime.def
```

Service revisions are resolved before submission. Each build copies its launcher,
build scripts, base-image pins and smoke test into a private recipe directory;
subsequent syncs cannot alter queued builds. Sources, Cargo targets, temporary files
and staging are isolated. Only dependency-download caches are shared. Generic
runtime builds freeze the definition; any external `%files` inputs remain
caller-managed and should be immutable. Existing runtime outputs are never replaced.

Rust/Node/Debian images are pinned by digest, Rust uses `--locked` and a generic
x86-64 target, and compiler/package versions are retained in the image. Apt
repositories can change, so this is not byte-for-byte reproducibility.

Before publication, GammaBoard starts its bundled PostgreSQL, API, frontend and
two workers and completes a 32-sample run. GammaLoop checks CLI startup. Failed
builds never change image selection. Successful service images live at
`images/<family>/<source-sha>/<image-sha256>/<family>.sif` with `image.json` metadata.
`select-image` changes only the default for new deployments; `up --image IMAGE_JSON`
can choose a published image without changing that default. Active jobs retain
their pinned digest. An older build finishing later cannot replace your selection.

## Supervision and recovery

The control allocation supervises both the container and its request launcher.
`status` includes the launcher's last success/error and the control heartbeat.
Healthy workers read that heartbeat instead of polling Slurm. Stale-heartbeat
scheduler checks are cached across workers, while each launcher pass batches its
scheduler/accounting queries. Missing accounting records and scheduler outages
remain unknown, not evidence that a job ended.

For manual diagnosis or a single reconciliation pass:

```bash
python3 ubelix.py watch-requests --once
```

Launch cards use the grouped schema:

```toml
[[groups]]
count = 1
name_prefix = "gpu"
max_start_failures = 6
config = { gpus = "h100:1", cpus_per_task = 4, mem = "16G" }
```

The server normalizes resources and reserves names. CPU aliases become
`cpus_per_task`; typed GPUs/GRES enable `--nv` and supply GPU capabilities.
Conflicting aliases/GPU declarations and simultaneous `mem`/`mem_per_cpu` are
rejected. Additional numeric group fields remain custom worker capabilities.
Slurm profiles accept `account`, `partition`, `qos`, `wckey`, `reservation`, `gres`,
`gpus`, `cpus_per_task`, `mem`, `mem_per_cpu`, `time`, `constraint`, `nodelist`, `exclude`.

Each launch has a renewable database lease and per-worker submission intent and
receipt. Worker registration is authoritative. Recovery adopts an unambiguous
matching Slurm job; an uncertain submission is never blindly repeated:

```bash
python3 ubelix.py resolve-submission 123 worker-1 --job-id 456789
python3 ubelix.py resolve-submission 123 worker-1 --not-submitted
python3 ubelix.py resolve-control --job-id 456790
python3 ubelix.py resolve-control --not-submitted
```

`--not-submitted` is your assertion after inspection, not an inference from missing
accounting. Worker repair requires a running control allocation. If a reconciliation pass
currently owns the request lease, retry after that pass releases it; active lease
ownership prevents competing repairs. See
[launch leases](docs/cluster-launches.md) for the database/API contract.

`down` fences new submissions, requests application suspension, and waits for
workers. Remaining owned worker jobs are cancelled and confirmed absent before
control is cancelled. It then confirms terminal control state. `down --force` skips
the API step but retains ownership and scheduler checks. Unknown control identity,
failed scheduler queries and cancellation timeouts remain errors; retry after
inspection. Reset/prune do not treat an unknown job as safely stopped.

## Deployment acceptance check

Use `resources/templates/runs/installation-smoke.toml`, then a small campaign with
six external workers, `max_active_runs = 4` and `min_evaluators_per_run = 2`. Expect
two simultaneous children and four evaluators. Exercise pause/resume and a worker
restart, check the shared sample cap, then run one representative physics example.
Existing process runtimes must match the current SDK/protocol; preserving runtime
files does not make older adapters compatible.

Local tests cover the helper's lifecycle and failure paths with fake Slurm and
container execution. Real Slurm scheduling, Apptainer startup, cluster resource
policies and the scientific runtime/state must be checked on UBELIX.
