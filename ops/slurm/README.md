# Slurm deployment helper

`slurm.py` is a reusable Slurm + Apptainer helper. GammaBoard itself has no Slurm
backend. The helper owns allocations, job submission/recovery, image builds and
site setup; GammaBoard owns runs, worker assignments, exports and graceful shutdown.
UBELIX-specific settings live in `examples/ubelix.toml`.

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
gb --instance /shared/demo/control/active.toml worker launch --count 6 --wait
gb --instance /shared/demo/control/active.toml run list
gb --instance /shared/demo/control/active.toml run create /shared/demo/run.toml \
  --start --max-evaluators 4 --export-path /shared/demo/results/result.json
```

Worker submissions are asynchronous; `--wait` waits for registration. Repeating
a worker submission creates additional workers. Repeating `up` while its control job is queued/running reuses the recorded deployment; uncertain
scheduler outcomes require reconciliation rather than blind resubmission.

The host-facing active file is a convenience copy. Each worker receives its own
immutable deployment generation's `worker.toml` database configuration. Configuration
and image hash checks prevent queued jobs from picking up later changes.

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

## Shutdown and recovery

```bash
gb --instance /shared/demo/control/active.toml stop --wait
python3 ops/slurm/slurm.py --config /shared/demo/slurm.toml down --wait
```

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

`install-examples` refreshes the five portable CPU examples and their shared child
card without touching existing database runs or private templates. CPU/GPU worker
cards remain external launch recipes; see the [canonical examples](../../docs/quickstart.md).

## Validation

The test suite uses a fake scheduler to exercise receipts, uncertain submission,
ownership checks, configuration/environment snapshots, image publication and
shutdown. Run `python3 -m pytest ops/slurm/tests`. Actual cluster operation still
requires a site-specific deployment smoke test; the tests do not submit real jobs.
