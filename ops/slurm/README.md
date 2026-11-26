# Slurm deployment

Use `python3 ops/slurm/slurm.py` for allocations, image builds and scheduler cleanup.
Use `gb` for runs, worker requests and results. The helper talks to GammaBoard's
HTTP API; workers use PostgreSQL directly. All commands below run from the repo root.

## UBELIX quick start

You need Python **3.11.8+**, Slurm and Apptainer. On UBELIX, load a suitable Python
module; Apptainer is already on PATH ([site documentation](https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/container-jobs/)).
Keep the checkout on shared storage and initialize its sibling workspace:

```bash
module load Python
python3 --version
apptainer --version
python3 ops/slurm/slurm.py init
```

For a checkout named `gammaboard`, this creates `../gammaboard-workspace/` containing
`slurm.toml`, `gammaboard.toml` and `environment.sh`. Paths in these files are relative
to the file containing them, so the default layout needs no path substitutions.
The SSH username comes from the login account, not the storage directory name.
Initialization refuses to overwrite existing files or put generated configuration
inside the checkout. If your workspace already exists, skip `init`.

| Override | How |
| --- | --- |
| Workspace | `init --workspace /shared/another-workspace` |
| Login username | `init --username other-login` |
| Complete SSH target | `init --ssh-target user@login.example` |
| Other cluster | `init --site generic`, then edit site resource settings |

Configuration selection is `--config FILE`, then `GAMMABOARD_SLURM_CONFIG`, then
`<checkout-name>-workspace/slurm.toml` beside the checkout, independently of the
current directory. For a custom workspace, select its config on later commands:

```bash
export GAMMABOARD_SLURM_CONFIG=/shared/another-workspace/slurm.toml
python3 ops/slurm/slurm.py up
```

Review account/partition/QoS and the Python module for your allocation. These live
in `slurm.toml` and `environment.sh`; application paths and ports live in
`gammaboard.toml`. `access.ssh_target` can also be edited directly. Copied templates
that specify only `access.ssh_host` infer the current login username when used.

The examples permit compute-node database/API access on a trusted cluster network.
For authentication, configure `[server.auth]` in the instance and `admin_password`
in the helper file; protect both files. See [deployment settings](../../docs/deployment.md).

Build the image, then start the control allocation:

```bash
python3 ops/slurm/slurm.py build gammaboard
# Wait for the workspace's artifacts/builds/BUILD_ID/build.json to say "completed".
python3 ops/slurm/slurm.py up --startup-timeout 1800
```

Builds use committed sources and run in Slurm. Check the printed job ID with
`squeue`/`sacct`; logs are under the workspace's `logs/slurm/`. Failed builds retain the
previous image. `up` prints a client connection file, SSH tunnel command and dashboard
URL. Run the tunnel command **on your laptop**, then open that URL. Forwarded
loopback ports can change without reconfiguring the default browser-origin policy.

## Run and inspect

Use the prepared client inside the image—no compilation or service startup on the
login node. Adjust these paths if your instance uses different locations:

```bash
# Convenience for the client examples; the helper itself needs no environment variable.
GB_WORK=$(realpath "../$(basename "$PWD")-workspace")
gb() {
  apptainer exec --cleanenv --bind "$GB_WORK" "$GB_WORK/images/gammaboard.sif" \
    gb --instance "$GB_WORK/control/active.toml" "$@"
}
gb worker launch --count 6 --wait --timeout 10m
gb run create "$GB_WORK/resources/templates/runs/tetrahedron-native.toml" --start
gb run wait tetrahedron-native --timeout 5m
gb run create "$GB_WORK/resources/templates/runs/tetrahedron-unstratified.toml" --start
gb run wait tetrahedron-unstratified --timeout 5m
```

With authentication enabled, use `gb auth login --password-stdin --output TOKEN_FILE`
and pass `--token-file TOKEN_FILE` afterward. Worker registration may wait on Slurm;
repeating `worker launch` requests additional workers. Run starts retry unavailable
workers for 10 seconds by default. `run wait` defaults to successful completion.
See the [CLI reference](../../docs/local-cli.md) for other operations.

Leave the allocation running for dashboard inspection, then stop it:

```bash
python3 ops/slurm/slurm.py down --wait
```

This finishes accepted work and confirms scheduler cleanup. `gb stop --wait` also
requests application shutdown, but `down` additionally confirms allocation exit.
Allocation expiry triggers graceful shutdown; it does not automatically resubmit.
Use `up --restore-workers` to restore saved workers in a later allocation.

## Update and maintain

Stop the deployment before updating its helper or replacing its image:

```bash
python3 ops/slurm/slurm.py down --wait
git pull
python3 ops/slurm/slurm.py build gammaboard
# Wait for the build record to say "completed".
python3 ops/slurm/slurm.py up
```

Rebuilding keeps the helper, client and service together. `install-examples`
refreshes the bundled run cards without changing database runs or private templates.
Use `gb db backup --output FILE` for backups; perform [offline database maintenance](../../docs/local-cli.md#database-maintenance)
inside an allocation with the deployment file and explicit process override.

For a queued deployment or unavailable API:

```bash
python3 ops/slurm/slurm.py status
python3 ops/slurm/slurm.py down --force
```

Forced cleanup verifies ownership and stops worker allocations before the control
allocation. Unknown scheduler/submission outcomes require reconciliation with
`resolve-control` or `resolve-submission`; see their `--help`. Repeating `up` reuses
a recorded queued/running deployment when its configuration matches.

## Browser-origin reload

Loopback HTTP(S) origins on any port are trusted by default. For exact-only access,
set `server.allow_loopback_origins = false` and configure `allowed_origins`.
After editing the **source instance file**, apply just those settings with:

```bash
python3 ops/slurm/slurm.py reload-origins
```

The helper copies browser policy into the control snapshot and calls the same API
as `gb config reload --origins`. Plain `gb` would only reread that generated snapshot.
Rejected changes restore the previous file. After an uncertain response, inspect
`gb status` and retry. Other configuration changes require a new allocation.

## Implementation and validation

- `slurm.py`: command entry point and orchestration.
- `gbops/`: configuration, scheduler, deployment, process and build logic.
- `jobs/`: shared batch entry point, image recipes and build/smoke scripts.
- `examples/`: editable configuration and CPU/GPU worker request templates.
- `tests/`: scheduler, build, recovery and process tests.

Jobs resolve configuration paths to absolute paths and freeze their helper,
environment, execution configuration and image identity.
`job_env_file` is optional; when set, it is copied, verified and sourced before the
job starts. It prepares the host environment, not container packages or automatic
variable forwarding through `--cleanenv`. Queued jobs ignore the submitting shell's
config selection. Only the control snapshot's browser policy is reloadable.

## Validation

Run `python3 -m pytest ops/slurm/tests`. Tests exercise scheduler failures, frozen
jobs, image publication and cleanup; they do not submit actual UBELIX jobs.
