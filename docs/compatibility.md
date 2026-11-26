# Formats and deployment compatibility

Install matching service, client, workers, dashboard, process SDK and deployment
helper revisions. Package versions alone do not establish format compatibility.

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
| Result export/import bundle | 2 |

[Runtime checks](../src/compatibility.rs) reject unsupported formats. Database
model 8 accepts additive [migrations](../migrations) after its supported baseline.
An incompatible model or migration history requires matching software or a fresh
database; there is no general converter. Database backups do not include external
sampler artifacts or generated physics states.

## Upgrade procedure

1. Check the format versions and pinned physics dependencies. Pause/checkpoint or
   finish active work, back up the database and external artifacts, then stop the
   instance. Retain the software and environments needed to read the backup.
2. Install matching components. Rebuild process-worker environments and deployment
   images, and regenerate GammaLoop states when required by the pinned revision.
3. Apply supported migrations with `gb local db migrate`, then restart the
   deployment. For an incompatible model, create a fresh database and recreate
   runs from validated cards.
4. Validate run cards and external workers before starting production work:

   ```bash
   gb run validate run.toml
   gb local run validate run.toml --probe
   ```

Local probes belong on the intended execution host. The
[evaluator checker](../process_api/integration.md) and
[sampler checker](../process_api/README.md#check-a-sampler) exercise process workers
without a service or database. Use clean Python package builds and run
`python -m pip check` in each worker environment.

## Contract references

- [CLI reference](local-cli.md): commands, file ownership and remote/local scope.
- [Management API](management-api.md): receipts, retries and shutdown.
- [Process protocol](../process_api/protocol.md): negotiated capabilities, encodings
  and callback compatibility.
- [Execution model](execution-model.md#tasks-checkpoints-and-failures): checkpoint
  recovery and external artifact ownership.
- [Backup and recovery](operations.md#backup-and-recovery): operational recovery.

Historical changes belong in the [changelog](../CHANGELOG.md); dated measurements
and validation reports describe the revisions they tested.
