# Process-worker tutorials

These are small extension examples, separate from the
[dashboard catalog](../../resources/templates/README.md). They use the same
process protocol and SDK as real integrations.

| Tutorial | Demonstrates | Requirements |
| --- | --- | --- |
| [Python evaluator](python_evaluator/README.md) | Unweighted values, logging and discrete projections | Python + SDK |
| [Python sampler](python_sampler/README.md) | Generation, feedback, recovery, PDF and exact discrete masses | Python + SDK |
| [Python observables](python_observables/README.md) | Per-sample histogram bundles from a synthetic polynomial | Python + SDK; GammaBoard's `gammaloop` feature |
| [Rust evaluator](rust_evaluator/README.md) | Direct JSON-RPC and ragged batches without the Python SDK | Rust; optionally Apptainer |
| [Rust GammaLoop wrapper](rust_gammaloop/README.md) | Native/process comparison, complex values and observable bundles | GammaLoop-enabled wrapper; values-only host needs no GammaLoop |

From the repository root, install the Python SDK in the environment used by the
workers:

```bash
python3 -m pip install ./process_api/python
```

The Python cards have a `python` replacement, defaulting to `python3`. Set it to
the absolute interpreter path when using a virtual environment. That interpreter
and the example source must be accessible to every assigned worker. The commands
refer to the source checkout relative to the default resource root.

Create a selected run with `./gb run create PATH`, then assign workers.
Tutorials are not automatically added to the dashboard menu or database.

Before integrating a real evaluator, read the [integration guide](../integration.md)
and run `check_evaluator.py` with valid physical points. Example paths are relative
to the configured resource root; use absolute command/artifact paths when copying
cards into an independently located deployment. An explicit `cwd` remains optional.
