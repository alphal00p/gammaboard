# Formats and deployment compatibility

GammaBoard supports the current configuration and execution interfaces. It does
not convert old definitions, databases, sampler state, or queue payloads. Start
from the current [templates](../resources/templates) and validate definitions:

```bash
gammaboard run validate run.toml
gammaboard run validate run.toml --probe
```

| Contract | Current version |
| --- | --- |
| GammaBoard and Python SDK package | 0.3 |
| Process protocol | `gammaboard-jsonrpc-v9` |
| Database model | 6 |
| Sampler checkpoint / saved sampler envelope | 5 |
| Queue envelope | `GBQ` followed by byte `0x04` |
| Performance responses | 2 |

These versions identify separate contracts. Unknown configuration fields and
unsupported protocol/state versions fail explicitly. The version checks protect
persisted data; they are not adapters for older formats. The single baseline
`202610080001_initial_schema.sql` includes campaign reservations, durable external
launch leases/receipts, and deployment/build fencing. It is for fresh databases;
previous migration checksums and older model databases are intentionally unsupported.
No incremental migration or compatibility adapter is retained.

Use matching server, worker, SDK, and integration revisions. Stop all components
before an incompatible upgrade. Preserve the old executable, environments,
physics states, immutable sampler artifacts, and database backup together. Use a
fresh database for an incompatible model; an old backup remains usable only with
its matching software. An older executable cannot enforce a newer compatibility
guard. See [backup and recovery](operations.md#backup-and-recovery).

Install process adapters into their intended environment and rebuild container
images when changing the SDK:

```bash
python -m pip install ./process_api/python ./integrations/madgraph
# In an environment with the required scientific dependencies:
python -m pip install ./process_api/python ./integrations/madnis
python -m pip check
```

Build wheels from a clean checkout. Stale ignored `build/` directories can retain
outdated modules; check installed exports and the protocol, not just the version
number. Regenerate GammaLoop physics states with GammaBoard's pinned revision.

Rust task fields and serialized task fields use `sampler`; the execution worker
role remains `sampler_aggregator`. Run lifecycle values include `completed` and
`failed`, unassigned live workers report `idle`, and auth responses expose
`auth_enabled`. Performance consumers read sampler identity from `metrics.epoch`
and `metrics.node_uuid`.

Dashboard view preferences use the current `gammaboard.panel-values.v1.` storage
namespace. Retired unversioned preferences are ignored, so zoom/selection/color
preferences reset once after this cleanup. Numerical results and run definitions
are unaffected.

Protocol v9 passes full typed domains to Python callbacks, returns rich sampler
feedback with ragged requested/actual prefixes, renames `discrete_pdf` to
`discrete_mass`, and removes `global_pdf_norm`. Prefix-keyed Voronoi centres,
axis roles, and partial feedback change persisted formats. Use matching workers
and a fresh database; no old state or protocol adapters are provided.

The statistics-display revision adds partition diagnostics, retained stratum-labelled
extrema, estimator identity and optional pooled comparisons to persisted outputs.
Pooled standard errors use the sample-variance convention and are unavailable for
fewer than two samples. These state changes require a fresh model-6 database and
model-5 checkpoints. The sampler feedback wire contract remains protocol v9 and
queue inputs retain envelope version 4.
