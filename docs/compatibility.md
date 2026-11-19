# Formats and deployment compatibility

GammaBoard supports its current configuration and execution interfaces. Validate
run cards against the installed version; use the [current templates](../resources/templates)
as starting points:

```bash
gammaboard run validate run.toml
gammaboard run validate run.toml --probe
```

## Version matrix

| Contract | Current version |
| --- | --- |
| GammaBoard and Python SDK package | 0.3 |
| Process protocol | `gammaboard-jsonrpc-v10` |
| Database model | 8 |
| Sampler checkpoint / saved sampler envelope | 6 |
| Queue envelope | `GBQ` followed by byte `0x04` |
| Performance responses | 2 |

These identify separate contracts. The package version alone does not establish
compatibility. [Runtime format checks](../src/compatibility.rs) reject unsupported
versions; [configuration validation](config.md) rejects unknown fields.
Database model 8 accepts the additive [migrations](../migrations) after its current
baseline. Databases with an older model or different baseline migration checksums
need matching older software or a fresh database; there is no general converter.

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

Use the profile-specific deployment procedure for [UBELIX](../ops/ubelix/README.md)
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
