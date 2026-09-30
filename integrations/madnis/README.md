# MADNIS GammaBoard API

MADNIS sampler implementation for GammaBoard using the
`gammaboard_process.run_sampler(...)` Python wrapper.

## Runtime Options

The sampler entrypoint defaults `OMP_NUM_THREADS` to `1` before importing the
numerical libraries, for all runtime options. Set it explicitly to use more CPU
threads within the sampler's allocated CPU budget.

### Direct venv

For local demos or machines where Apptainer is not available, install the
sampler directly into a virtual environment under this integration directory:

```bash
cd integrations/madnis   # from the gammaboard repo root

uv venv --python 3.13 --seed .venv
. .venv/bin/activate
python -m pip install ../../process_api/python .
```

Use this GammaBoard process command:

```toml
command = ["$resources/../integrations/madnis/.venv/bin/madnis-gammaboard-sampler"]
cwd = "$resources/.."
```

With `cwd = "$resources/.."`, sampler `save_path` values should be relative to
the GammaBoard workspace, for example:

```toml
save_path = "integrations/madnis/checkpoints/ghost_bump_madnis"
```

### Apptainer

Apptainer is the most portable path for UBELIX and other non-Nix systems:

```bash
apptainer build --force madnis.sif apptainer.def
```

The definition file builds from Git, not from the local checkout. Pin the exact
source when needed:

```bash
apptainer build --force \
  --build-arg GAMMABOARD_REF=<branch-or-commit> \
  madnis.sif apptainer.def
```

On UBELIX, run the build from the GammaBoard workspace:

```bash
python ubelix.py build apptainer integrations/madnis/madnis.sif integrations/madnis/apptainer.def
```

Nix is still supported where available:

```bash
nix build .#runtime
```

## Use With GammaBoard

`examples/ghost_bump_madnis.toml` is a ready-to-copy run template. It uses the
direct venv command by default and keeps Apptainer and Nix alternatives
commented next to it.

The sampler command uses `$resources/..` because GammaBoard expands
`$resources` to the default resource directory. Sampler `args` are passed
through unchanged, so `save_path` should be relative to the configured process
`cwd`.

The process entrypoint is:

```bash
python -u -m run_sampler
```

## Training feedback

GammaBoard delivers weighted feedback (`f/q`) in chunks that can differ from the
generation batches. The adapter waits for every sample in a minibatch, removes
the proposal weight before passing targets to MADNIS, and keeps the final training
barrier closed until the optimizer step completes. `training_updates` counts
completed steps; `total_trained_samples` is the existing generated-training-sample
counter. Feedback from subsequent inference samples is not buffered.

Run the adapter regressions from the repository root in the MADNIS environment:

```sh
PYTHONPATH=integrations/madnis/src:process_api/python/src \
  python -m unittest discover -s integrations/madnis/tests
```

The sampler and the installed SDK must both come from this checkout (protocol v3).
The Nix package also builds the SDK from this repository, so no separate SDK pin can drift.
