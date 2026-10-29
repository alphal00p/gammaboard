# UBELIX deployment

Run directly from a Git checkout on a UBELIX login node with Python 3.11.8+, Slurm
and Apptainer. One ignored `local.toml` contains your settings; a separate shared
workspace holds images, databases, jobs and physics data. There is no sync or image
selection step. A successful service build replaces its fixed image.

## First deployment

Load a recent Python module in your login shell before running the helper:

```bash
module load Python
python3 --version # Must be 3.11.8 or newer; the system python3 may be older.
```

If the default module is too old or cannot be loaded directly, use
`module spider Python`, then `module spider Python/VERSION` for a suitable version
and its loading instructions. See the
[UBELIX module guide](https://hpc-unibe-ch.github.io/software/Lmod_modules/).
Use the same `module load` command in `~/.config/gammaboard/slurm.env` so build,
control and worker jobs also get the required Python. The helper uses only the
standard library; no pip installation is needed.

```bash
git clone https://github.com/alphal00p/gammaboard.git
cd gammaboard/ops/ubelix
cp local.example.toml local.toml
chmod 600 local.toml # Restrict access because this file can contain admin_password; optional without secrets.
# Edit workspace_root and any cluster resource overrides in local.toml.
python3 ubelix.py build gammaboard
```

`build` submits a Slurm job and returns its job ID. Wait for it to finish: check
`squeue -j JOB_ID`, then the `state` in
`<workspace>/artifacts/builds/BUILD_ID/build.json` and logs under
`<workspace>/logs/slurm/build/`. Only `state = "completed"` means the new image is
ready. Then:

```bash
python3 ubelix.py up
python3 ubelix.py submit-workers --count 6 --prefix cpu
```

`up` waits for the API and managed worker launcher and prints an SSH tunnel command.
Run that tunnel on your workstation. No login-terminal watcher is needed. If the
startup wait expires while Slurm is still queueing the job, repeat `up` or use
`status`; the recorded job is reused.

## Configuration

Paths in `local.toml` are absolute or relative to that file, independent of your
shell's directory. The workspace must be on shared storage accessible to all jobs.
No workspace environment variable is needed or read by the operator commands.

```toml
workspace_root = "/storage/research/itp_localunitaritydata/YOUR_USER/gammaboard-workspace"
ssh_target = "YOUR_USER@submit03.unibe.ch" # optional; default is your user at submit03

[cluster.control]
time = "02:00:00"
cpus_per_task = 4
mem = "8G"

[cluster.build]
cpus_per_task = 8
mem = "24G"

[server]
name = "UBELIX"
```

`[cluster.control]`, `[cluster.worker]` and `[cluster.build]` override
[cluster defaults](config/cluster.toml); `[server]` overrides
[server defaults](config/server.toml); `[runtime]` supplies application runtime and
PostgreSQL tuning. Choose account, partition, QoS and time values allowed by your
allocation. Defaults use `gratis`/`epyc2`: control one hour, workers thirty minutes,
builds two hours. `up --time` overrides control walltime.

Ports, database URL and local/external worker mode belong to the helper. Use
`--port-offset N` consistently for another deployment and `up --single-node` for
workers inside the control allocation. Resource profiles and application settings
are frozen when a deployment starts. Editing `local.toml` takes effect on the next
deployment; a running allocation cannot be resized by repeating `up`.

For authentication, set `[server.auth]` as described in the
[deployment guide](../../docs/deployment.md#access)
and the top-level `admin_password` in `local.toml`. The managed launcher retains
its credential in a private deployment file. Treat both files as secrets.
`~/.config/gammaboard/slurm.env` is sourced at job start for site/module setup.

## Updates and clean redeployment

```bash
python3 ubelix.py down
git pull --ff-only
python3 ubelix.py build gammaboard
# Wait for the build record to say "completed".
python3 ubelix.py up
```

Stop **all deployments in the workspace** before rebuilding any service or process
runtime. The helper refuses builds while deployment jobs are queued/running, and
refuses startup or another build while a build job is queued/running. Use
`scancel JOB_ID` to cancel a build. Jobs in other workspaces are independent.

Builds use the checked-out commit, including local commits, and reject dirty source
trees. Sources and job scripts are captured before submission, so later checkout
changes do not alter queued jobs. Compilation and smoke-test failures preserve the
previous image. Successful builds automatically replace `images/gammaboard.sif`
(or `images/gammaloop.sif`) and its `.json` provenance metadata. There is no image
history; rollback means checking out an older commit and rebuilding.

For an incompatible [database model](../../docs/compatibility.md), reset after
stopping and before the next `up`:

```bash
python3 ubelix.py reset-db --discard
# Or archive the old database instead of deleting it:
python3 ubelix.py reset-db
```

`up --fresh-db` also archives the stopped database before starting fresh. Reset is
scoped to the selected workspace/offset and ordinary paths under `resources/db`.
Archives keep data and configuration, **not the old image**; restoring requires
rebuilding compatible software. `down --force` supports an unavailable API while
still checking job ownership and waiting for cancellation.

For the old synced installation, stop its watchers, deployments and build jobs
using its existing tools first. Then clone this repository and use an empty
workspace. Nothing needs migrating if you are discarding the old deployment.
There is no automatic whole-workspace wipe.

## Workspace and templates

```text
images/                   gammaboard.sif, gammaboard.json; optionally gammaloop.*
deployments/              job records, configuration and launcher snapshots
resources/db/             PostgreSQL data
resources/templates/      editable dashboard templates
resources/states/         generated physics states
resources/runtimes/       process environments and custom images
artifacts/builds/          build records, recipes, logs and failed build evidence
artifacts/download-cache/ shared dependency downloads
artifacts/database-archives/
logs/slurm/
```

On first deployment, dashboard templates are copied from the checkout and enriched
with the UBELIX node cards. Later starts preserve that directory, including edits
and deletions made in the dashboard. To refresh bundled templates, copy individual
cards from the checkout, or remove `resources/templates/` while stopped to reseed
it entirely at the next start. Generated states and process runtimes are supplied
separately. They must match the current SDK, protocol and physics code.

Successful builds remove their source and temporary staging automatically. Old
build directories and retired deployment directories can be deleted manually when
no longer needed. Keep the current deployment record referenced by each
`deployments/active-*.json` pointer, even while stopped. There is no automatic
retention/pruning policy.

## GammaLoop and custom runtimes

For the GammaLoop CLI image, clone GammaLoop on shared storage and set top-level
`gammaloop_source` in `local.toml` to that checkout. Then run
`python3 ubelix.py build gammaloop`. It uses that checkout's clean `HEAD` and replaces
`images/gammaloop.sif`. Choose a revision compatible with your physics states.

For a custom process image:

```bash
python3 ubelix.py build apptainer resources/runtimes/custom.sif /path/to/runtime.def
```

The output is relative to the workspace, the definition to your shell directory.
The definition is captured before submission; external `%files` inputs remain
caller-managed. Existing custom runtime files are never replaced implicitly;
remove the old file while stopped to rebuild it.

Service base images are pinned by digest; Rust builds use `--locked` and generic
x86-64 flags. Before publication, GammaBoard starts PostgreSQL, API, frontend and
two workers and completes a 32-sample run; GammaLoop checks CLI startup. Apt
repositories can change, so these are not byte-for-byte reproducible builds.

## Workers and recovery

Use dashboard node cards or `submit-workers`. Group settings override the worker
profile; CPU capability reflects the effective CPU allocation. Declare GPUs in
request groups so the server derives matching capabilities and NVIDIA bindings.
Custom numeric capabilities are retained. See the
[launch contract](../../docs/cluster-launches.md) for the grouped schema and leases.

Control and workers receive a 180-second termination warning and have a bounded
120-second process shutdown. New worker walltimes are capped to the remaining
control lifetime when Slurm supplies it. Healthy workers use a shared control
heartbeat; stale-heartbeat scheduler checks are cached. `status` includes launcher
health. Shutdown stops new submissions, drains/cancels workers, then stops control.

Ambiguous submissions remain recorded rather than being submitted twice. Inspect
Slurm, then use `resolve-control --job-id ID` or `--not-submitted`; worker repair uses
`resolve-submission REQUEST_ID NODE --job-id ID` or `--not-submitted`. A manual
`watch-requests --once` performs one reconciliation pass. These are recovery tools;
normal operation needs only `build`, `up`, `status`, worker requests and `down`.

## Deployment acceptance check

In the dashboard, run `installation-smoke`, then a small campaign with six workers,
`max_active_runs = 4` and `min_evaluators_per_run = 2`. Expect two simultaneous
children and four evaluators. Exercise pause/resume and a worker restart, check the
shared sample cap, then run a representative physics example.

Local tests simulate Slurm and container execution. Actual Apptainer builds,
UBELIX policies and scientific runtimes still require this cluster check.
