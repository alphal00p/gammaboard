# Documentation

| Task | Guide |
| --- | --- |
| Install and run examples | [Setup and examples](quickstart.md) |
| Find commands and understand local/remote scope | [CLI reference](local-cli.md) |
| Configure instances, access and hosting | [Deployment](deployment.md) |
| Define runs, tasks, tuning and campaigns | [Configuration reference](config.md) |
| Use the dashboard, inspect results and recover work | [Operations](operations.md) |
| Understand strata, weights and uncertainties | [Stratification](stratification.md) |
| Check supported formats and upgrade components | [Compatibility](compatibility.md) |
| Measure throughput and overhead | [Benchmarking](benchmarking.md) |

Deployment profiles: [local](../ops/local/README.md),
[ITPhlies](../ops/itphlies/README.md), and [Slurm/Apptainer](../ops/slurm/README.md)
with a UBELIX example.

## Extend GammaBoard

| Task | Guide |
| --- | --- |
| Automate management through HTTP | [Management API](management-api.md) |
| Implement sampling, evaluation, feedback and recovery | [Execution model](execution-model.md) |
| Configure external process workers | [Process-worker guide](../process_api/README.md) |
| Integrate and check an external evaluator | [Evaluator integration](../process_api/integration.md) |
| Browse small worker implementations | [Process tutorials](../process_api/examples/README.md) |
| Implement the process transport | [Wire protocol](../process_api/protocol.md) |
| Implement an external worker launcher | [Launch contract](cluster-launches.md) |

The [dashboard template catalog](../resources/templates/README.md),
[GammaLoop preparation tool](../tools/prepare-gammaloop/README.md), and
[stratification benchmark](../benchmarks/stratification/README.md) have their own
references. Templates become database runs only when explicitly created.

[Historical validation and benchmark records](validation/README.md) retain their
original revisions, settings and results. Use the guides above for current behavior.
