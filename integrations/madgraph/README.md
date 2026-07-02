# MadGraph7 GammaBoard Integration

Thin Python process-evaluator wrapper for MadGraph7/MadSpace states.

The wrapper is intentionally narrow. For one selected MG7 subprocess it builds:

1. a flat `madspace.PhaseSpaceMapping`
2. a `madspace.DifferentialCrossSection`
3. the minimal MadSpace callable needed to evaluate their product on batches

It uses MG7's `build_integrands` helper to construct those objects, but bypasses
the resulting integrand's sampler, adaptive mapping, discrete samplers, channel
weights, and multichannel phase-space machinery.

## Reproducible Install

The examples are pinned to MadGraph7 commit
`920d224232b24a3a736443986e2040369929298f` (2026-06-28). MadSpace is part of
that repository and must be built from the same checkout. Do not install the
released MadSpace wheel or run `pip install .` in `madspace/`; either can select
different build settings or an ABI that does not match MG7.

Run these commands from the GammaBoard repository root:

```bash
git clone https://github.com/MadGraphTeam/MadGraph7.git ../MadGraph7
git -C ../MadGraph7 checkout --detach 920d224232b24a3a736443986e2040369929298f

python ../MadGraph7/madspace/install.py \
  --source --no-cuda --no-hip --no-simd --no-debug

python -m venv integrations/madgraph/.venv
integrations/madgraph/.venv/bin/python -m pip install --upgrade pip
integrations/madgraph/.venv/bin/python -m pip install --force-reinstall \
  integrations/madgraph
```

The development shell sets `MAKEFLAGS` to override the `/bin/bash` path
hardcoded by generated MG7 makefiles with Nix's Bash path. Start the GammaBoard
server and workers from `nix develop` so evaluator-time compilation inherits
this setting.

MadGraph7 itself is not a pip dependency. Provide its root path via
`madgraph_root` in the evaluator config. The wrapper adds both
`<madgraph_root>` and `<madgraph_root>/madspace/install` to `sys.path`.

MadGraph7 moves fast: a generated `mg7_state` is tied to the MG7 version that
wrote it. Generate the state with the pinned checkout and use that same checkout
and its `madspace/install` directory at runtime.

## Runtime Flow

GammaBoard sends batches of points in `[0, 1]^N`, where `N` is the callable
dimension reported by `MadSpaceState.random_dim` and equals
`PhaseSpaceMapping.random_dim()`. The wrapper maps those points to phase space,
evaluates the differential cross section, and returns one component: `weight`.

Use a vector accumulator with component `weight`.

## Evaluator Config

```toml
[evaluator]
kind = "process_evaluator"
command = ["$resources/../integrations/madgraph/.venv/bin/madgraph-gammaboard-evaluator"]
cwd = "$resources/.."
domain = { continuous = { dims = 7 } }   # must equal MadSpaceState.random_dim
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

After completing the pinned install above, run:

```bash
cd /path/to/gammaboard   # the gammaboard repo root
mkdir -p integrations/madgraph/artifacts/pp_eej
cat > /tmp/pp_eej_mg7.cmd <<'EOF'
import model sm
generate p p > e+ e- j
output integrations/madgraph/artifacts/pp_eej/mg7_state -f
quit
EOF
python ../MadGraph7/bin/mg5_aMC /tmp/pp_eej_mg7.cmd
```

For this pinned revision and process, `random_dim` is `7`.

## Experiments

- [`experiments/gammaloop_vs_madgraph`](experiments/gammaloop_vs_madgraph/README.md):
  LO e+ e- -> d d~ cross section checked against GammaLoop, with run configs for
  both engines.

## Native Event Generation

The `madgraph-gammaboard-event-evaluator` process evaluator runs an existing
state's native `bin/generate_events -f` command. MG7 remains responsible for
survey, adaptation, multichannel generation, unweighting, channel combination,
and event files.

The evaluator reads the new run's `Events/<run>_NN/info.json` and returns:

- the MG7 cross-section estimate and uncertainty;
- generated and unweighted event counts plus native timings;
- every histogram configured in the state's `Cards/run_card.toml`;
- a GammaLoop-compatible observable bundle rendered by the existing backend.

Use it with `accumulator = "gammaloop"` and exactly one zero-dimensional
trigger sample. The empty point makes explicit that MG7 ignores GammaBoard
sample coordinates and owns native event generation:

```toml
[evaluator]
kind = "process_evaluator"
command = ["$resources/../integrations/madgraph/.venv/bin/madgraph-gammaboard-event-evaluator"]
cwd = "$resources/.."
domain = { continuous = { dims = 0 } }
accumulator = "gammaloop"

[evaluator.args]
state_path = "integrations/madgraph/artifacts/pp_eej/mg7_state"

[[task_queue]]
kind = "set_accumulator"
accumulator = "gammaloop"

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 1 }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 0 } }
```

[`examples/pp_eej_generate_events.toml`](examples/pp_eej_generate_events.toml)
reuses the same local generated state as the direct MadSpace evaluator example.
Event files remain under the state directory and are not stored in PostgreSQL.
