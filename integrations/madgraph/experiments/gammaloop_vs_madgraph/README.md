# GammaLoop vs MadGraph: e+ e- -> d d~ at LO

LO cross section of e+ e- -> d d~ at sqrt(s) = 1000 GeV computed with two
engines: GammaLoop (Local Unitarity) and MadGraph7/MadSpace (tree matrix
element x flat phase space).

Analytic photon-only value: `N_c * Q_d^2 * 4*pi*alpha^2/(3 s) = 3.096e-2 pb`
(alpha = 1/132.507).

## Configs

| file | engine | computes |
|------|--------|----------|
| `ee_ddx_gammaloop.toml` | GammaLoop | e+ e- -> gamma* -> d d~, photon only |
| `ee_ddx_madgraph.toml`  | MadGraph  | e+ e- -> d d~, Z forbidden (`generate ... / z`) |

Both run havana training (1e6 samples, samples_for_update = 5e4, 128 bins) then
havana inference (1e7 samples). Paths inside resolve against the gammaboard repo
root; run `gammaboard run create <file>` from there. Artifact build steps are in
each config header.

GammaLoop's `epem_a_ddx` is the photon self-energy supergraph (no Z). MadGraph's
default `e+ e- > d d~` includes the Z; the `/ z` removes it so both compute the
photon-only process.

## Results

Photon only:

| engine | pb |
|--------|----|
| GammaLoop | 3.1033e-2 +- 3.6e-5 |
| MadGraph  | 3.0922e-2 +- 8.8e-6 |
| analytic  | 3.096e-2 |

Defaults (GammaLoop photon-only vs MadGraph gamma+Z):

| run | pb |
|-----|----|
| GammaLoop (photon only) | 3.103e-2 |
| MadGraph (gamma + Z)    | 9.212e-2 |

e+ e- -> mu+ mu- at the same energy (MadGraph, gamma+Z): 1.0426e-1 pb; gamma+Z
tree value 1.047e-1 pb.

## Status

MadGraph7 is under rapid development. Rerunning will very likely break or need
adjustments to the wrapper (run-card schema and madspace ABI change between MG7
revisions; a state is tied to the MG7 version that wrote it). Last observed
break on `itphlies`: `GeneratorConfig has no attribute 'cut_efficiency_threshold'`
when the MG7 checkout is ahead of the installed madspace wheel; rebuild madspace
from that MG7 source with
`python ../MadGraph7/madspace/install.py --source --no-cuda --no-hip --no-simd --no-debug`.
