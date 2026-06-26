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

## Training vs inference

By default the sampler trains: it produces batches, ingests the matching
evaluator values, and updates the Havana grid, pausing once
`stop_training_after_n_samples` is reached.

**Inference mode** samples from a frozen grid: it never updates the grid and
needs no training values. Hand a trained grid over via a file — a training run
writes it (`save_path`), an inference run loads it (`grid_path`):

```toml
# 1) Training task: write the grid to a file on each checkpoint.
[[task_queue]]
name = "train"
kind = "sample"
stop_condition = { max_samples = 100000 }
sampler_aggregator.config = { kind = "process_sampler", command = [ ... ], cwd = "$resources",
  requires_training_values = true,
  args = { seed = 0, bins = 16, samples_for_update = 4096,
           stop_training_after_n_samples = 100000,
           save_path = "havana_demo.grid" } }

# 2) Inference task: load the frozen grid, no training values.
[[task_queue]]
name = "infer"
kind = "sample"
stop_condition = { max_samples = 10000000 }
sampler_aggregator.config = { kind = "process_sampler", command = [ ... ], cwd = "$resources",
  requires_training_values = false,
  args = { inference = true, grid_path = "havana_demo.grid", samples_for_update = 100000 } }
```

`save_path`/`grid_path` are relative to the sampler's `cwd`. In inference mode
`sample_plan` keeps producing (the task's `stop_condition` ends it),
`produce_latent_batch` does not retain samples, and `ingest_training_values` is
a no-op. A snapshot-restored sampler with `inference = true` in its `args` works
the same way, loading the grid from the snapshot instead of a file.
