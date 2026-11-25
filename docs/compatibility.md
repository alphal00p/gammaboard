# Formats and deployment compatibility

GammaBoard supports its current configuration and execution interfaces. Validate
run cards against the installed version; use the [current templates](../resources/templates)
as starting points:

```bash
gb run validate run.toml
gb local check-run run.toml --probe
```

## Version matrix

| Contract | Current version |
| --- | --- |
| GammaBoard and Python SDK package | 0.3 |
| Process protocol | `gammaboard-jsonrpc-v10` |
| Selected-output evaluator extension | `gammaboard-evaluation-v1` |
| Selected-output binary encoding | `gammaboard-evaluation-binary-v1` |
| Database model | 8 |
| Sampler checkpoint / saved sampler envelope | 6 |
| Queue envelope | `GBQ` followed by byte `0x04` |
| Performance responses | 2 |
| Management API | `/api/v1` |

These identify separate contracts. The package version alone does not establish
compatibility. [Runtime format checks](../src/compatibility.rs) reject unsupported
versions; [configuration validation](config.md) rejects unknown fields.
Database model 8 accepts the additive [migrations](../migrations) after its current
baseline. Databases with an older model or different baseline migration checksums
need matching older software or a fresh database; there is no general converter.

## Evaluator contract migration

Native providers implement `descriptor()` and `evaluate(batch, request, emit)`.
`eval_values` and `eval_samples` remain convenience callers, not the provider
implementation hooks. `SampleOutput.diagnostics` is now optional; absent diagnostics
must not be represented as a measured zero. This is a Rust source-interface change.

Process workers negotiate the evaluator extension during v10 initialization. New
hosts retain the fixed-output `eval_batch` adapter; old workers need no rewrite.
The Rust GammaLoop wrapper also retains its old-host interface. New Python providers
use `EvaluatorDescriptor`, `EvaluationRequest` and `EvaluationBatch`; legacy `eval`
classes remain supported, and `ObservableBatch` is an alias for `EvaluationBatch`.

New cards can omit evaluator `domain`/`output` and discover the descriptor, or supply
an explicit descriptor to avoid execution on the service host. These new fields
and discovered configurations require a matching host; they are not a downgrade
format for older releases. Existing explicit domain/output cards remain valid.
Task output selection comes from accumulators; unsupported capabilities fail before
execution. The [wire contract](../process_api/protocol.md#selected-output-evaluator-contract)
documents weights, details, diagnostics, discovery and both encodings.

No database, queue or sampler-checkpoint version changes accompany this extension.
Observable reduction still depends on GammaLoop until the shared model is extracted;
its imports now pass through one `evaluation::observables` boundary.

## Upgrade procedure

1. Check the format versions and integration revisions before changing binaries.
2. Stop servers and workers. Back up the database and retain the matching executable,
   environments, generated physics states and immutable sampler artifacts. Database
   backups do not include external files; see [backup and recovery](operations.md#backup-and-recovery).
3. Install matching server, worker, dashboard, SDK and deployment scripts. Update
   SDK installations inside process-worker environments and rebuild their images.
   Regenerate GammaLoop states when required by the revision pinned in GammaBoard.
4. For a supported database model, restart the deployment to apply its pending
   migrations. For an incompatible model, create a fresh database and recreate runs
   from validated cards. Old backups remain usable with their matching software.
5. Validate external workers and example definitions before resuming production
   work. The [sampler checker](../process_api/README.md#check-a-sampler) can test a
   process sampler without a server or database.

Use the profile-specific deployment procedure for [UBELIX](../ops/slurm/README.md)
or [ITPhlies](../ops/itphlies/README.md). Build Python wheels from a clean checkout:
stale ignored `build/` directories can retain outdated modules. Check installed
exports and the protocol, then run `python -m pip check` in each worker environment.

## Current upgrade notes

- Additive target-error and plot migrations preserve existing numerical results;
  omitted target uncertainties remain zero. Older snapshots can lack optional
  diagnostic fields; those values remain unavailable rather than being invented.
- The estimator-terminology migration changes saved `basic` comparisons to `pooled`,
  `objective_basic` to `objective_pooled`, and campaign estimator tags `combined`
  to `independent_sum`. Values/errors, raw accumulators, queues, sampler state,
  user parameters and histogram names are preserved. Old exported files are not
  rewritten; new JSON/CSV exports use the current names.
- Offline-worker actions require matching dashboard/server and UBELIX scripts.
  They preserve saved launch recipes and worker history; see the [worker model](operations.md#manage-work).
- Protocol v10 requires one unweighted evaluator result per input and per-sample
  observable bundles. Older aggregate-only adapters need updating; see the
  [callback contract](../process_api/README.md#python-callbacks).
- Checkpoint notifications are an optional v10 extension, negotiated at sampler
  initialization. Older workers are not sent the new RPC, and saved-state encoding
  is unchanged. See [initialization](../process_api/protocol.md#initialization).
- Database model 8/checkpoint model 6 retain conditional histogram bundles for each
  stratum. Older pooled-only histogram checkpoints cannot be upgraded to this
  estimator; start fresh with matching software.

## Unified service migration

Management now requires the control/API service. Use `gb --endpoint URL` or an
instance/connection file, rather than `--runtime-config` plus direct database
access. The service and optional dashboard share the API port; nginx and `deploy`
are no longer part of standard startup. Use `start --foreground` for process owners.
`run create --export-path` schedules an absolute server-side export; `run results
--output` downloads locally. Worker runtime configuration remains supported.
Rebuild Slurm images to include both `gammaboard` and `gb` and the updated smoke test.

Worker launches now submit backend-neutral requests (`worker launch --count N`).
Use `--wait` when later commands require registered workers. `start --workers N`
uses the same request model. Offline administration is under `gb local`; remote
management commands are no longer accepted by the `gammaboard` executable.
