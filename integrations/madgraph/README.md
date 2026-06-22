# MadGraph7 GammaBoard Integration

Python process-evaluator wrapper for matrix elements produced by
<https://github.com/MadGraphTeam/MadGraph7>.

MadGraph7 is still under active development, so this integration keeps the
GammaBoard boundary stable and narrow:

1. MadGraph7 generates or exposes a Python-callable matrix-element object.
2. GammaBoard maps unit-hypercube points to phase-space momenta.
3. The wrapper calls the matrix-element object from Python.
4. The wrapper returns matrix-element, Jacobian, and/or weighted value through
   the GammaBoard process evaluator protocol.

## Install

```bash
cd integrations/madgraph
python -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install .
```

## Matrix-Element Callable Contract

The first version expects a Python callable:

```python
def matrix_element(momenta, parameters=None):
    ...
```

where `momenta` has shape `(nr_samples, nr_particles, 4)` and contains incoming
particles first, then final-state particles. It must return one scalar per
sample.

This callable can be a small adapter around a MadGraph7-generated process
module. The wrapper adds `madgraph_root` and optional `search_path` entries to
`sys.path` before importing it.

## Example Evaluator Config

```toml
[evaluator]
kind = "process_evaluator"
command = ["$resources/../integrations/madgraph/.venv/bin/madgraph-gammaboard-evaluator"]
cwd = "$resources/.."
domain = { continuous = { dims = 2 } }
components = ["value", "matrix_element", "phase_space_weight"]

[evaluator.args]
ecm = 91.188
phase_space = { kind = "two_body", final_state_masses = [0.0, 0.0], include_flux = true }
matrix_element = { kind = "python_callable", module = "my_mg7_adapter", function = "matrix_element", search_path = "integrations/madgraph/artifacts/ee_mumu" }
```

## Next Extension Point

Once the exact MadGraph7 callable API stabilizes, add a `kind = "madgraph7"`
backend in `madgraph_gammaboard.artifacts` that builds/loads the callable from
`model`, `process`, and `cache_dir`. The GammaBoard-facing evaluator config does
not need to change.
