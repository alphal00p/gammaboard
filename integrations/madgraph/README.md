# MadGraph7 GammaBoard Integration

Thin Python process-evaluator wrapper for MadGraph7/MadSpace states.

The wrapper is intentionally narrow. For one selected MG7 subprocess it builds:

1. a flat `madspace.PhaseSpaceMapping`
2. a `madspace.DifferentialCrossSection`
3. the minimal MadSpace callable needed to evaluate their product on batches

It does not use MG7's `build_integrands` helper, Vegas/adaptive mappings,
discrete samplers, channel weights, or multichannel phase-space machinery.

## Install

```bash
cd integrations/madgraph
python -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install --force-reinstall .
```

MadGraph7 itself is not a pip dependency. Provide its root path via
`madgraph_root` in the evaluator config. The wrapper adds both
`<madgraph_root>` and `<madgraph_root>/madspace/install` to `sys.path`.

## Runtime Flow

GammaBoard sends batches of points in `[0, 1]^N`, where `N` is the callable
MadSpace dimension reported by `MadSpaceState.random_dim`. This can be larger
than `PhaseSpaceMapping.random_dim()` because MadSpace may add direct
non-adaptive choice variables around the mapping/cross-section pair. The wrapper
maps those points to phase space, evaluates the differential cross section, and
returns one component: `weight`.

Use a vector accumulator with component `weight`.

## Evaluator Config

```toml
[evaluator]
kind = "process_evaluator"
command = ["$resources/../integrations/madgraph/.venv/bin/madgraph-gammaboard-evaluator"]
cwd = "$resources/.."
domain = { continuous = { dims = 8 } }   # must equal MadSpaceState.random_dim
components = ["weight"]

[evaluator.args]
state_path = "integrations/madgraph/artifacts/pp_eej/mg7_state"
madgraph_root = "/path/to/MadGraph7"   # omit if MG7 is already on sys.path
subprocess_index = 0
flavor_index = 0
output = ["weight"]
```

## Finding `random_dim`

```python
from madgraph_gammaboard.state import MadSpaceState

state = MadSpaceState.load(
    state_path="integrations/madgraph/artifacts/pp_eej/mg7_state",
    madgraph_root="/path/to/MadGraph7",
    subprocess_index=0,
    flavor_index=0,
)
print(state.random_dim)
print(state.mapping_random_dim)
```

Set `domain.continuous.dims` in the run config to `state.random_dim`.

## Generating a MG7 State

```bash
git clone https://github.com/MadGraphTeam/MadGraph7.git /tmp/MadGraph7
cd /path/to/gammaboard   # the gammaboard repo root
mkdir -p integrations/madgraph/artifacts/pp_eej
cat > /tmp/pp_eej_mg7.cmd <<'EOF'
import model sm
generate p p > e+ e- j
output integrations/madgraph/artifacts/pp_eej/mg7_state -f
quit
EOF
python /tmp/MadGraph7/bin/mg5_aMC /tmp/pp_eej_mg7.cmd
```

## Experiments

- [`experiments/gammaloop_vs_madgraph`](experiments/gammaloop_vs_madgraph/README.md):
  LO e+ e- -> d d~ cross section checked against GammaLoop, with run configs for
  both engines.
