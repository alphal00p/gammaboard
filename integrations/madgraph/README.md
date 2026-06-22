# MadGraph7 GammaBoard Integration

Python process-evaluator wrapper for matrix elements produced by
<https://github.com/MadGraphTeam/MadGraph7>.

MadGraph7 is still under active development, so this integration keeps the
GammaBoard boundary stable and narrow:

1. MadGraph7 generates a matrix-element artifact outside GammaBoard.
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

## Runtime Flow

For the current `2 -> 2` evaluator, GammaBoard sends points in `[0, 1]^2`.
The wrapper maps each point to incoming and outgoing four-momenta, evaluates the
MadGraph matrix element, multiplies by the two-body phase-space weight, and
returns the selected components.

Available output components are:

- `value`: `matrix_element * phase_space_weight`
- `matrix_element`: raw matrix-element value returned by the backend
- `phase_space_weight`: mapping weight, including flux when requested

Use a vector accumulator with matching component names.

## Supported Matrix-Element Backends

### `kind = "madgraph_f2py_subprocess"`

Recommended for the current MadGraph7 CLI workflow. It loads a generated
standalone subprocess module such as:

```text
SubProcesses/P1_epem_mupmum/matrix2py*.so
```

Example:

```toml
[evaluator.args.matrix_element]
kind = "madgraph_f2py_subprocess"
search_path = "integrations/madgraph/artifacts/ee_mumu/ee_mumu_f2py/SubProcesses/P1_epem_mupmum"
module = "matrix2py"
initialize_path = "integrations/madgraph/artifacts/ee_mumu/ee_mumu_f2py/Cards/param_card.dat"
function = "py_matrix_sum"
flavor = [1, 1, 2, 2]
helicities = "all_pm"
normalization = 4.0
```

`py_matrix_sum` is a wrapper-side helper for flavor-dependent artifacts: it
calls generated `py_matrix(p, helicity, flavor)` for every configured helicity
and divides by `normalization`. Direct `py_smatrix` calls are intentionally
rejected because some generated F2PY wrappers call the Fortran routine with the
wrong argument list and can crash the worker process. `py_get_value` is still
available for generated artifacts whose wrapper exposes the full needed
signature correctly.

### `kind = "python_callable"`

Loads an arbitrary Python callable:

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

`artifacts/ee_mumu/my_mg7_adapter.py` is a committed non-physical demo adapter
so the example can start before a real MadGraph7 artifact exists. Replace it for
real physics runs.

### `kind = "madgraph_python_matrix"`

Loads a real MadGraph Python-export matrix class, i.e. a module containing a
generated `Matrix_*` class with:

```python
matrix.smatrix(p, model, flavor=None)
```

where `p` is the MadGraph `(4, nexternal)` momentum layout. GammaBoard passes
momenta internally as `(nr_samples, nexternal, 4)` and the wrapper transposes
each point before calling MadGraph.

Example:

```toml
[evaluator.args.matrix_element]
kind = "madgraph_python_matrix"
search_path = "integrations/madgraph/artifacts/ee_mumu_python"
module = "matrix_ee_mumu"
class = "Matrix_epem_mupmum"
model_module = "models.sm.parameters"
pdgs = [11, -11, 13, -13]
```

If `class` is omitted, the wrapper accepts the module only when it contains
exactly one `Matrix_*` class.

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

Once the exact MadGraph7 process-builder API stabilizes, add a builder command
that generates either a Python-export module or an F2PY module under
`integrations/madgraph/artifacts/...`. The evaluator should continue to consume
generated artifacts rather than generating code on every worker start.
