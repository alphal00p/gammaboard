# MadGraph7 GammaBoard Integration

Thin Python process-evaluator wrapper for MadGraph7 processes, using the
[`madspace`](https://github.com/MadGraphTeam/MadGraph7/tree/main/madspace)
Python API from MG7.

The split is deliberately simple:

1. Generate a MG7 process state outside GammaBoard.
2. The wrapper loads the state via `MadSpaceState`, which builds the combined
   phase-space + matrix-element integrand using MG7's `madspace` runtime.
3. GammaBoard sends unit-hypercube points; the wrapper returns the integrand
   weight through the GammaBoard process evaluator protocol.

## Install

```bash
cd integrations/madgraph
python -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install .
```

MadGraph7 itself is not a pip dependency. Provide its root path via
`madgraph_root` in the evaluator config. The wrapper automatically adds
both `<madgraph_root>` (for the `madgraph` package) and
`<madgraph_root>/madspace/install` (for the pre-built `madspace` extension)
to `sys.path`.

## Runtime Flow

GammaBoard sends batches of points in `[0, 1]^N` where `N` equals the
integrand's `random_dim`. The wrapper calls the MadSpace runtime, which maps
those points to phase-space momenta, evaluates the matrix element, and returns
the combined weight. The only output component is `weight`.

Use a vector accumulator with the `weight` component name, or a scalar
accumulator if only one component is listed.

## Evaluator Config

```toml
[evaluator]
kind = "process_evaluator"
command = ["$resources/../integrations/madgraph/.venv/bin/madgraph-gammaboard-evaluator"]
cwd = "$resources/.."
domain = { continuous = { dims = 6 } }   # must equal the integrand's random_dim
components = ["weight"]

[evaluator.args]
state_path = "integrations/madgraph/artifacts/ee_mumu/mg7_state"
madgraph_root = "/path/to/MadGraph7"   # omit if MG7 is already on sys.path
subprocess_index = 0   # which subprocess to use (default 0)
phase_space = "flat"   # "flat" or "multichannel"
channel_index = 0      # which integrand channel (default 0)
device = "cppnone"     # "cppnone", "cuda:<idx>", "hip:<idx>", or "state"
thread_count = -1      # -1 = auto
output = ["weight"]
```

### Finding `random_dim`

The number of integration variables depends on the process and phase-space
type. For a hadronic 2→2 flat phase space it is typically 6 (2 Bjorken-x + 4 angles). To check at runtime:

```python
from madgraph_gammaboard.state import MadSpaceState
state = MadSpaceState.load(state_path="...", madgraph_root="...")
print(state.random_dim)
```

Set `domain.continuous.dims` in the run config to match this value.

## Generating a MG7 State

```bash
git clone https://github.com/MadGraphTeam/MadGraph7.git /tmp/MadGraph7
cd /home/user/Workspace/repos/gammaboard
mkdir -p integrations/madgraph/artifacts/ee_mumu
cat > /tmp/ee_mumu_mg7.cmd <<'EOF'
import model sm
generate e+ e- > mu+ mu-
output integrations/madgraph/artifacts/ee_mumu/mg7_state -f
quit
EOF
python /tmp/MadGraph7/bin/mg5_aMC /tmp/ee_mumu_mg7.cmd
```

The resulting directory is what `state_path` should point to.

## Device Selection

| `device` value | Backend |
|---|---|
| `cppnone` (default) | C++ CPU, single thread |
| `state` | reuse context from the loaded process |
| `cuda:<idx>` | CUDA GPU (requires CUDA build of MG7) |
| `hip:<idx>` | HIP/ROCm GPU (requires HIP build of MG7) |
