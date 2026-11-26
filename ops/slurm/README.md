# Slurm deployment helper

`slurm.py` is a reusable Slurm + Apptainer helper. GammaBoard itself has no Slurm
backend. The helper owns allocations, job submission/recovery, image builds and
site setup; GammaBoard owns runs, worker assignments, exports and graceful shutdown.
UBELIX-specific settings live in `examples/ubelix.toml`. The helper handles scheduler
operations; use `gb` for runs, worker requests, results and application shutdown.

## UBELIX quick start

On a UBELIX login node, clone the repository and select a writable **shared**
directory for deployment data. Keep configuration outside the checkout: image
builds require a clean committed source tree.

```bash
git clone https://github.com/alphal00p/gammaboard.git
cd gammaboard
module load Python
python3 --version             # must be 3.11.8 or newer
apptainer --version
export GB_WORK=/storage/research/itp_localunitaritydata/$USER/gammaboard
mkdir -p "$GB_WORK"
cp ops/slurm/examples/ubelix.toml "$GB_WORK/slurm.toml"
cp ops/slurm/examples/gammaboard.toml "$GB_WORK/gammaboard.toml"
cp ops/slurm/examples/environment.sh "$GB_WORK/environment.sh"
```

Choose a shared path you can access; the storage path above is an institute-specific
example. Edit the copied files: replace all `YOUR_USER` and `/shared/...` paths,
make `workspace_root`, `image`, `binds` and the instance paths agree, set
`access.ssh_target`, and choose account/partition/QoS limits available to you.
Put the same working Python module command in `environment.sh`. UBELIX provides
Apptainer on PATH without a module; see its official [container guide](https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/container-jobs/)
and [module guide](https://hpc-unibe-ch.github.io/software/Lmod_modules/).
Keep `externally_managed = true`. The supplied cluster bind addresses permit
compute-node access; authentication and exact-only browser origins can be configured
in the instance file for a shared deployment.

Submit the build, wait for its record to say `completed`, then start the control job:

```bash
python3 ops/slurm/slurm.py --config "$GB_WORK/slurm.toml" build gammaboard
# Inspect the printed job with squeue/sacct and artifacts/builds/BUILD_ID/build.json.
python3 ops/slurm/slurm.py --config "$GB_WORK/slurm.toml" up --startup-timeout 1800
```

The build and service run in Slurm allocations. `up` prints a `client_instance`, an
SSH tunnel command and a dashboard URL. Run that SSH command **on your laptop**,
then open the printed localhost URL. Changing its local port works with the default
loopback-origin policy and does not require a new allocation.

Use the lightweight client already inside the image; this function starts no
service or worker processes on the login node and does not compile anything:

```bash
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

Adjust the function's image, binds and active-file path if you chose different
locations. For authenticated instances, log in with `gb auth login --password-stdin
--output "$GB_WORK/session.token"`, then pass `--token-file "$GB_WORK/session.token"`
to subsequent commands. Worker registration can wait on the Slurm queue; `run start`
only retries unavailable workers for 10 seconds by default. `run wait` defaults to
successful completion. Omit `--max-evaluators` to use compatible free capacity.
Leave the allocation running for dashboard inspection, then finish with:

```bash
python3 ops/slurm/slurm.py --config "$GB_WORK/slurm.toml" down --wait
```

Use `up --restore-workers` to restore saved worker requests in a later allocation.
No automatic resubmission occurs when an allocation expires.


## Configuration

Keep two files:

- A GammaBoard instance file with `externally_managed = true` and explicit paths,
  database, server, logging and shutdown settings. Start from `examples/gammaboard.toml`.
- A helper file selected by `--config FILE`, containing `instance_file`,
  `workspace_root`, `image`, `job_env_file`, explicit container `binds`, control/
  worker/build resource profiles and optional SSH access settings. Start from
  `examples/generic.toml` or `examples/ubelix.toml`.

Paths resolve against the file containing them, independent of the shell directory.
Use shared storage accessible to the control and worker jobs. Configure container
binds for all required host paths. The application image contains the service,
lightweight CLI and optional dashboard at `/usr/local/share/gammaboard/dashboard`. Configure API and database ports in the
instance file.

`job_env_file` is optional. When configured it must exist. Its contents are copied
into the deployment/build snapshot, hashed, verified and sourced by the batch job
before starting Python or Apptainer. There is no implicit home-directory environment
file. A module loaded here prepares the host job environment; it does not install
software inside a container or automatically forward variables through `--cleanenv`.

The helper needs Python 3.11.8+, Slurm commands and Apptainer. The control/worker
profiles must use resources permitted by your site. It does not compile GammaBoard
on the login node merely to submit jobs. Protect helper files containing credentials
and generated private deployment files appropriately.

## Build and start

From the repository checkout, with the site Python environment available:

```bash
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml build gammaboard
# Wait for the recorded build job to complete successfully.
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml up
```

`build` submits a job and returns its ID. Its record is under
`<workspace>/artifacts/builds/BUILD_ID/build.json`; Slurm logs are under
`<workspace>/logs/slurm/`. Build jobs freeze the source revision, helper scripts,
resource profile, environment and output destination before submission. Successful
validated builds replace the configured application image. Failed builds retain
the previous image. Stop deployments sharing the build workspace before replacing
images; per-instance manifests are registered with that workspace for this check.

`up` submits one control allocation, starts GammaBoard inside it with the explicit
`--allow-local-processes` override, and waits for readiness. Worker jobs use the
same override inside their allocations. The shared instance file
continues to prohibit accidental execution elsewhere.

Configure cluster-reachable database/server bind addresses. Use the supplied
examples on a trusted cluster network, or configure database authentication.

The helper prints `client_instance=.../active.toml` after readiness. This generated
file contains the allocated node's API endpoint, without database credentials.
Use the prepared `gb` client on the login node for ordinary operations:

```bash
gb --instance /shared/demo/control/active.toml worker launch --count 6 --wait --timeout 10m
gb --instance /shared/demo/control/active.toml run list
gb --instance /shared/demo/control/active.toml run create /shared/demo/run.toml \
  --start --export-path /shared/demo/results/result.json
```

Worker submissions are asynchronous; `--wait` waits for registration. Repeating
a worker submission creates additional workers. Repeating `up` while its control job is queued/running reuses the recorded deployment; uncertain
scheduler outcomes require reconciliation rather than blind resubmission.

The host-facing active file is a convenience copy. Each worker receives its own
immutable deployment generation's `worker.toml` database configuration. Configuration
and image hash checks prevent queued jobs from picking up later execution changes.
Only browser-origin fields in the control instance snapshot can be reloaded;
worker configuration, database paths, resource profiles, environment and image
remain pinned.

## One API for management

The helper claims launch requests, renews leases, reports receipts, reads shutdown
status, and acknowledges cleanup through HTTP. It connects directly to
the service API port; no frontend proxy or dashboard is required. It verifies
`deployment_id` and `build_id` before authenticating. Scheduler submission remains
in the helper, and worker computation continues to access PostgreSQL directly.

Ordinary `gb` clients use that same service. With authentication enabled, obtain a
personal token using `gb auth login --password-stdin --output TOKEN_FILE` and pass
`--token-file TOKEN_FILE`. The helper retains its frozen launcher credential and
reconnects/re-authenticates on errors.

## Browser-origin reload

Loopback HTTP(S) browser origins on any port are accepted by default. Set
`server.allow_loopback_origins = false` with an exact `allowed_origins` list to opt
out. The helper never adds forwarding origins behind that policy.

The Slurm service reads a generated deployment snapshot, so editing the source
instance file followed by plain `gb config reload --origins` would reread the old
snapshot. After editing the source instance file, use:

```bash
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml reload-origins
```

This copies **only** the two origin settings into the running control instance file
and invokes the same `reload_origins` management API as `gb config reload --origins`.
All execution settings remain protected by their digest, and queued workers retain
their original configuration. Rejected policy changes restore the previous file;
an ambiguous transport failure retains the candidate file so the operator can
inspect `gb status` and safely retry. Origin changes do not alter allocation identity.
Use this explicit reload command to apply edits before reusing `up`. Other deployment
changes require shutdown and a new allocation.

## Shutdown and recovery

```bash
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml down --wait
```

Alternatively, `gb --instance /shared/demo/control/active.toml stop --wait` requests
application shutdown through the API; `down` additionally confirms scheduler cleanup.

Shutdown requests and draining state are durable in PostgreSQL. The service waits
for accepted work and exports, checkpoints workers, and requests external cleanup.
The helper cancels owned queued jobs, allows running workers to exit, cancels any
remaining owned allocations, and acknowledges the exact shutdown ID through HTTP.
The service retains PostgreSQL until cleanup is acknowledged.

`gb stop` waits for application shutdown completion. The helper's `down` also
confirms that the scheduler allocations have ended. A lost API connection before
acknowledgment is not considered successful shutdown. The service leaves an
owner-side receipt in its explicit state directory; clients need not mount it.

Use forced scheduler cleanup when the control job is queued or the API is unavailable:

```bash
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml status
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml down --force
```

Forced cleanup verifies deployment/job ownership and confirms worker allocations
are gone before canceling the control job. `resolve-control` and
`resolve-submission` handle uncertain submissions; scheduler lookup errors are not
proof that a job ended. Allocation expiry triggers graceful application shutdown
within the signal margin. Allocations are not automatically resubmitted.

Backups use `gb db backup --output LOCAL_FILE`. Perform [offline database
maintenance](../../docs/local-cli.md#database-maintenance) inside an appropriate
allocation using the deployment instance file and explicit process override.

## Example runs

`install-examples` refreshes the six portable CPU examples and their shared child
card without touching existing database runs or private templates. CPU/GPU worker
cards remain external launch recipes; see the [canonical examples](../../docs/quickstart.md).

## Validation

The test suite uses a fake scheduler to exercise receipts, uncertain submission,
ownership checks, configuration/environment snapshots, image publication and
shutdown. Run `python3 -m pytest ops/slurm/tests`. Actual cluster operation still
requires a site-specific deployment smoke test; the tests do not submit real jobs.
