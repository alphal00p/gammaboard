# GammaBoard process SDK

Python interfaces and runners for external evaluators, samplers, materializers and
transforms using `gammaboard-jsonrpc-v10`. Install from this directory into the
environment containing your worker's scientific dependencies:

```bash
python -m pip install .
```

Use the SDK from the same GammaBoard revision. The [process-worker guide](../README.md)
is the reference for setup, Python callbacks, snapshots, examples and the standalone
sampler checker. The [execution model](../../docs/execution-model.md) defines runtime
ownership and recovery; the [wire protocol](../protocol.md) specifies encoding for
workers in any language.
