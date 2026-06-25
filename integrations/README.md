# GammaBoard Integrations

This directory contains optional wrappers and heavier examples for external
tools. The default local resource space stays in `resources/`; integrations are
opt-in and may require separate Python environments, containers, GPUs, or large
external artifacts.

- `madnis/`: MADNIS process sampler wrapper.
- `madgraph/`: MadGraph7/MadSpace process-evaluator wrapper, with example run
  configs and the GammaLoop comparison experiment.
