# Python Symbolica Havana Sampler

This example uses Nix to pin Symbolica and NumPy for a reproducible sampler
runtime.

```bash
cd process_api/examples/python_sampler_symbolica_havana
PYTHONPATH=../../python/src:src nix shell .#runtime -c python -u -m run_sampler
```

The worker entrypoint is `src/run_sampler.py`. It imports
`SymbolicaHavanaSampler` and hands it to `gammaboard_process.run_sampler(...)`.
Custom samplers should follow the same shape: implement the class, then call
`run_sampler(MySampler)` from a tiny entrypoint module.

Fresh initialization calls:

```python
MySampler(discrete_cardinalities=..., continuous_dims=..., **args)
```

Sampler restore calls `from_snapshot(...)` when a persisted sampler checkpoint
is present.
