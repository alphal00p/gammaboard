# Changelog

## 0.2.0 - Unreleased

- Added persisted distributed sampler/evaluator execution with live dashboard
  panels, provenance, and CPU-hour accounting.
- Added parameter scans, hyperparameter tuning, and variance-directed
  integration campaigns with persistent child runs and derived observables.
- Added the `gammaboard-jsonrpc-v2` process runtime protocol and Python helpers
  for evaluators, samplers, materializers, and batch transforms.
- Added GammaLoop histogram support, complex observables, and optional MadGraph
  and MADNIS integrations.
- Consolidated local, ITPhlies, and UBELIX deployment workflows.
- Added database backup/restore commands, explicit version reporting,
  reproducible UBELIX revision selection, and release upgrade/ops CI coverage.
- Simplified managed PostgreSQL to an explicit passwordless trust model;
  `--postgres-trusted-network` limits remote trust to directly connected networks.
