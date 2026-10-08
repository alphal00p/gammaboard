# GammaBoard Docs

This directory is intended to be self-contained enough to copy into an
organization wiki.

Start with [quickstart.md](quickstart.md), then use the topic pages as needed:

- [requirements.md](requirements.md): supported environment and reproducibility metadata.
- [examples.md](examples.md): shared examples, dependencies, and expected workload sizes.
- [benchmarking.md](benchmarking.md): sparse frontier, process overhead, targeted comparisons and measurement rules.
- [performance-development.md](performance-development.md): current results, portable plots and optimization evidence.
- [sampling.md](sampling.md): generation sizing, evaluator splitting, training barriers and checkpoint ownership.
- [config.md](config.md): runtime, server, deploy, run, task, and node config.
- [deployment.md](deployment.md): shared deploy model, profiles, paths, images, and ports.
- [operations.md](operations.md): auth, node/run lifecycle, logs, and recovery.
- [recovery-testing.md](recovery-testing.md): backend suite, seeded fault injection, invariants and publication claim boundaries.
- [process-runtime.md](process-runtime.md): external process evaluator/sampler protocol.
- [frontend.md](frontend.md): dashboard architecture and panel data flow.

Repository-local operator notes that are not copied into this directory:

- `ops/ubelix/README.md`: UBELIX Slurm/Apptainer workflow.
- `ops/itphlies/README.md`: ITPhlies profile notes.
- `process_api/README.md`: process API examples and wrappers.
