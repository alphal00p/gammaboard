# Python Symbolica Havana sampler

The [run card](run.toml) trains a grid, resets the accumulator, and runs frozen-grid
inference. Set up the [sine evaluator](../python_scalar_sin/README.md) first;
the sampler uses its pinned Nix runtime automatically.

From the repository root, with a deployment and available workers:

```bash
./gammaboard run create process_api/examples/python_sampler_symbolica_havana/run.toml
./gammaboard run resume python-sampler-symbolica-havana-demo --max-evaluators 1
```

| Sampler argument | Purpose |
| --- | --- |
| `samples_for_update` | Training-window size |
| `stop_training_after_n_samples` | Training sample budget |
| `bins`, `initial_training_rate`, `final_training_rate` | Grid adaptation |
| `save_path` | Export the trained grid on checkpoints |
| `inference = true`, `grid_path` | Load a frozen grid without training feedback |

Grid paths are relative to the sampler's `cwd`. Use distinct output paths for
independent runs and preserve any grid still needed by an inference task.
The sampler's recovery snapshot embeds its grid and pending training samples.

Implementation: [demo_sampler.py](src/demo_sampler.py).
Shared callback, feedback, and restoration rules: [process guide](../../README.md).
