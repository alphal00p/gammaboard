# Documentation

## Get started

| Task | Guide |
| --- | --- |
| Install and run the standard examples | [Setup and examples](quickstart.md) |
| Define runs, tasks, sources, tuning and campaigns | [Configuration reference](config.md) |
| Understand strata, weights and reported errors | [Stratification](stratification.md) |

## Operate GammaBoard

| Task | Guide |
| --- | --- |
| Assign workers, inspect plots, compare observables and recover work | [Operations](operations.md) |
| Configure hosting, access, ports and capacity | [Deployment](deployment.md) |
| Upgrade matching binaries, SDKs and saved formats | [Compatibility](compatibility.md) |
| Measure throughput and overhead | [Benchmarking](benchmarking.md) |

Deployment profiles: [local](../ops/local/README.md),
[ITPhlies](../ops/itphlies/README.md), and [UBELIX](../ops/ubelix/README.md).

## Extend GammaBoard

| Task | Guide |
| --- | --- |
| Implement sampling, materialization, evaluation, feedback and recovery | [Execution model](execution-model.md) |
| Write or test an external worker | [Process-worker guide](../process_api/README.md) |
| Browse small worker implementations | [Process tutorials](../process_api/examples/README.md) |
| Implement the transport without the Python SDK | [Wire protocol](../process_api/protocol.md) |
| Submit workers through an external scheduler | [Cluster launch contract](cluster-launches.md) |

[Historical validation and benchmark records](validation/README.md) retain their
original revisions, settings and results. Use the guides above for current behavior.

The [dashboard catalog](../resources/templates/README.md),
[GammaLoop preparation tool](../tools/prepare-gammaloop/README.md), and
[stratification benchmark](../benchmarks/stratification/README.md) have separate
purposes; none automatically creates database runs when the server starts.
