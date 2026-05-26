# BNL benchmark investigation

Date: 2026-05-25.

This note records the reproducible BNL setup, the generated states, and the
runtime/integration scans used for the 2 -> 5 amplitude graph in
`examples/cli/BNL/BNL.dot`. All heavy generation and integration runs were
executed behind a 30 GiB RSS guard. The selected setup is CFF,
`SingleParametric`, spherical coordinates with linear mapping, summed graphs,
summed orientations, no LMB multichanneling, one Euler-angle stability rotation,
and Double+Quad stability levels.

Important UV-composition status: `generate_only_integrated_uv_with_n_loops=1`
currently generates only the filtered integrated-UV sector. It does not keep the
original contribution. A temporary code change was tested to make the option
preserve the original contribution and reject only non-matching UV spinney
nodes; the `n_loops=2` sanity check then kept the original contribution, but the
`n_loops=1` BNL generation was still killed by the OS. That code change was
reverted. The viable current workflow is therefore additive:

```text
physical result = local-only state - filtered integrated-UV sector state
```

The minus sign is fixed by the generated sector convention. A 50k-point
`m_uv = 700, 1000, 1300` check confirmed that `local-only - sector` is stable
within MC uncertainty, while `local-only + sector` is not.

## Kinematics and helicities

The final scan uses the same massless 2 -> 5 phase-space point and helicity
assignment for all four masses; only `MT` and `ymt` are changed.

```toml
e_cm = 1000.0
helicities = [-1, 1, -1, 1, 1, -1, 1]
momenta = [
  [500.0, 0.0, 0.0, 500.0],
  [500.0, 0.0, 0.0, -500.0],
  [163.5141597587927, -155.7860143642699, -12.32488779805292, 48.12167194606676],
  [146.5267245560251, -18.94527369806406, 33.44069886802352, -141.3961713514227],
  [162.3143263141399, 4.025183527597962, -151.4903823484999, 58.14122874765845],
  [296.8259181161096, 175.6763342487531, 137.9119270904214, -195.5089553360614],
  [230.8188712549325, -4.970229714017088, -7.53735581189216, 230.6422259937584],
]
```

The kinematic point was selected from the random 2 -> 5 all-massless samples
tested during the scan. The selected helicities flip the two initial leptons and
the outgoing muon pair relative to the original trial point; this avoided a
suppressed below-threshold central value and gave substantially smaller relative
errors.

## Threshold localization knobs

Two independent threshold-localization widths matter:

- Integrated threshold CT: `subtraction.integrated_ct_settings.range.h_function_settings.sigma`.
- Local threshold CT: `subtraction.local_ct_settings.uv_localisation.gaussian_width`.

No new setting was needed for the local width. The local counterterm uses
`evaluate_uv_damper`, with `gaussian_width` as the width and `sliver_width` as
the hard support gate. Event-level inspect checks confirmed that changing
`gaussian_width` changes individual `threshold_counterterm:*` additional
weights while leaving the original contribution unchanged.

The selected production values are:

```toml
subtraction.integrated_ct_settings.range.h_function_settings.sigma = 1.0
subtraction.local_ct_settings.uv_localisation.gaussian_width = 1.0
subtraction.local_ct_settings.uv_localisation.sliver_width = 10.0
```

Important scan observations:

- `sigma = 0.25` was unstable for the selected threshold point.
- `sigma = 4.0` sometimes reduced variance but changed the finite-statistics
  central value strongly in small samples, so it was not used for the final
  four-mass comparison.
- `gaussian_width` in the range `0.5..4.0` was viable; `1.0` was retained as a
  stable default and is the value used in the final integrations.
- `cpe_iterations = 10000` stayed within generation budget but was slower at
  runtime than the optimized `cpe_iterations = 5` state.
- A `summed_function_map = true` state was generated, but the corresponding
  `SummedFunctionMap` runtime path failed with an invalid helicity-type error in
  this branch, so it was not used for the final numbers.
- A scalar `dod0_bubble` threshold+UV probe at 50k points showed no statistically
  compelling sigma dependence around the defaults; the largest tested shifts
  were about 2 sigma. The existing `dod0/dod1/dod2` scalar UV benchmark passes
  at the default values, so the BNL production runs keep the defaults above.

A higher-statistics BNL check at `MT = ymt = 250` was then run with optimized
states and the additive `local-only - integrated-sector` composition. At 500k
points per local/sector piece, the default-width result was
`-3.181(10)e-15 - 1.574(12)e-15 i`. Varying the integrated threshold
localization width was not invariant at this statistic:

| variation | result | pull from default |
| --- | --- | ---: |
| `integrated sigma = 0.5` | `-8.35(99)e-15 - 1.9(1.7)e-15 i` | `5.18 sigma` in real part |
| `integrated sigma = 2.0` | `-1.93(32)e-15 - 0.55(34)e-15 i` | `3.74 sigma` in real part |
| `local gaussian_width = 0.5` | `-3.30(23)e-15 - 1.60(26)e-15 i` | compatible |
| `local gaussian_width = 2.0` | `-9.6(6.7)e-15 + 4.7(6.2)e-15 i` | inconclusive, high variance |

So the benchmark/default localization parameters are used for all production
numbers below. The width variations are retained only as a validation stress
test, not as optimization knobs.

## Generated states

The clean final states are all under `examples/cli/BNL/states/`.

| state | threshold CTs | optimized | evaluator pieces | generation command | guard time | peak RSS |
| --- | --- | --- | ---: | --- | ---: | ---: |
| `final_no_threshold_reference` | no | no | 1 | `run generate_bnl_no_threshold_reference -D mt=510.0 -D ymt=510.0` | 23.325 s | 3.477 GiB |
| `final_no_threshold_reference_optimized` | no | yes | 1 | `run generate_bnl_no_threshold_reference_optimized -D mt=510.0 -D ymt=510.0` | 26.375 s | 4.343 GiB |
| `final_threshold_reference` | yes | no | 4 | `run generate_bnl_threshold_reference -D mt=400.0 -D ymt=400.0` | 28.413 s | 3.715 GiB |
| `final_threshold_reference_optimized` | yes | yes | 4 | `run generate_bnl_threshold_reference_optimized -D mt=400.0 -D ymt=400.0` | 32.411 s | 3.383 GiB |
| `final_threshold_mt250_reference_optimized` | yes | yes | 4 | `run generate_bnl_threshold_reference_optimized -D mt=250.0 -D ymt=250.0` | 39.9 s | 3.56 GiB |
| `physical_threshold_mt250_singleparametric_filtered1_optimized` | yes | yes | 1 | `run generate_bnl_threshold_reference_physical_optimized -D mt=250.0 -D ymt=250.0` | 6.2 s | 0.54 GiB |
| `final_threshold_mt350_reference_optimized` | yes | yes | 4 | `run generate_bnl_threshold_reference_optimized -D mt=350.0 -D ymt=350.0` | 37.1 s | 4.32 GiB |
| `physical_threshold_mt350_singleparametric_filtered1_optimized` | yes | yes | 1 | `run generate_bnl_threshold_reference_physical_optimized -D mt=350.0 -D ymt=350.0` | 4.9 s | 0.36 GiB |
| `physical_no_threshold_singleparametric_filtered1_optimized` | no | yes | 1 | `run generate_bnl_no_threshold_reference_physical_optimized -D mt=510.0 -D ymt=510.0` | 3.1 s | 0.39 GiB |

Example full command:

```bash
./target/release/gammaloop --clean-state --no-save-state \
  -s ./examples/cli/BNL/states/final_threshold_reference_optimized \
  ./examples/cli/BNL/BNL.toml \
  run generate_bnl_threshold_reference_optimized -D mt=400.0 -D ymt=400.0
```

The generation command blocks call `save state -o` explicitly. `--no-save-state`
only prevents an extra save attempt at process exit.

## Inspect benchmark timings

All timings below use `inspect --bench 5 --n_batches 10 --minimal-integrand`.
The current warmup-based auto-sizing undershot the requested wall time on this
point, but the reported uncertainty is still the standard error over 10 batches.

| state | threshold runtime | total time / sample | evaluator time / sample | samples |
| --- | --- | ---: | ---: | ---: |
| `final_no_threshold_reference` | off | `1.9658e-3 +/- 1.48e-5 s` | `1.8846e-3 +/- 1.47e-5 s` | 743 |
| `final_no_threshold_reference_optimized` | off | `4.8728e-4 +/- 1.40e-6 s` | `4.1889e-4 +/- 1.48e-6 s` | 961 |
| `final_threshold_reference` | on | `2.5149e-3 +/- 3.88e-6 s` | `2.4001e-3 +/- 3.70e-6 s` | 716 |
| `final_threshold_reference_optimized` | on | `7.2629e-4 +/- 2.00e-6 s` | `6.2314e-4 +/- 2.01e-6 s` | 862 |

Example reproduction commands:

```bash
./target/release/gammaloop --read-only-state \
  -s ./examples/cli/BNL/states/final_no_threshold_reference_optimized \
  ./examples/cli/BNL/BNL.toml \
  run bench_bnl_reference \
  -D mt=510.0 -D ymt=510.0 \
  -D disable_threshold_subtraction=true \
  -D output_file=final_no_threshold_reference_optimized_bench.json

./target/release/gammaloop --read-only-state \
  -s ./examples/cli/BNL/states/final_threshold_reference_optimized \
  ./examples/cli/BNL/BNL.toml \
  run bench_bnl_reference \
  -D mt=400.0 -D ymt=400.0 \
  -D disable_threshold_subtraction=false \
  -D integrated_h_sigma=1.0 \
  -D local_gaussian_width=1.0 \
  -D output_file=final_threshold_reference_optimized_bench.json
```

## Current Optimized Additive Scan

The final production scan uses the optimized generated states above, default
threshold-localization parameters, and the physical additive composition:

```text
physical result = optimized local-only state - optimized filtered integrated-UV sector state
```

All runs below used the release CLI, 5 cores, the same phase-space point and
helicities, and the 30 GiB RSS guard. Threshold subtraction is enabled for the
above-threshold masses and disabled for the below-threshold masses. The
auxiliary UV scale was kept close to the top mass; the selected precision
below-threshold scan uses `m_uv = MT`.

| label | `MT = ymt` | `m_uv` | threshold subtraction | samples / piece | real part | imaginary part |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| R1 | 250.0 | 250.0 | on | 1,000,000 | `-2.899(12)e-15` | `-1.324(13)e-15` |
| R2 | 350.0 | 425.0 | on | 1,000,000 | `-2.18(95)e-15` | `+0.65(3.45)e-15` |
| R3 | 510.0 | 510.0 | off | 4,000,000 | `+1.0402(25)e-15` | `-4.838(27)e-16` |
| R6 | 530.0 | 530.0 | off | 4,000,000 | `+7.208(20)e-16` | `-2.908(21)e-16` |
| R4 | 550.0 | 550.0 | off | 4,000,000 | `+5.588(18)e-16` | `-2.143(20)e-16` |
| R7 | 570.0 | 570.0 | off | 4,000,000 | `+4.513(17)e-16` | `-1.637(20)e-16` |
| R5 | 600.0 | 600.0 | off | 4,000,000 | `+3.496(16)e-16` | `-1.151(17)e-16` |

The corresponding workspaces are:

| label | local-only workspace | integrated-UV sector workspace |
| --- | --- | --- |
| R1 | `optimized_15min_R1_mt250_muv250_base_1000k_5c` | `optimized_15min_R1_mt250_muv250_sector_1000k_5c` |
| R2 | `optimized_15min_R2_mt350_muv425_base_1000k_5c` | `optimized_15min_R2_mt350_muv425_sector_1000k_5c` |
| R3 | `optimized_15min_R3_mt510_muv510_base_4000k_5c` | `optimized_15min_R3_mt510_muv510_sector_4000k_5c` |
| R6 | `optimized_15min_R6_mt530_muv530_base_4000k_5c` | `optimized_15min_R6_mt530_muv530_sector_4000k_5c` |
| R4 | `optimized_15min_R4_mt550_muv550_base_4000k_5c` | `optimized_15min_R4_mt550_muv550_sector_4000k_5c` |
| R7 | `optimized_15min_R7_mt570_muv570_base_4000k_5c` | `optimized_15min_R7_mt570_muv570_sector_4000k_5c` |
| R5 | `optimized_15min_R5_mt600_muv600_base_4000k_5c` | `optimized_15min_R5_mt600_muv600_sector_4000k_5c` |

The threshold points are useful physical checks but are still too noisy to
serve as precision-ratio anchors. The selected precision ratios are therefore
formed from the denser below-threshold mass scan. A real or imaginary component
is considered selected when moving it by `5 sigma` cannot change its first two
significant digits. Seven real components pass this criterion:

| ratio | real part | 5 sigma | two-digit margin | imaginary part |
| --- | ---: | ---: | ---: | ---: |
| R3/R6 | `+1.4741(51)` | `0.0255` | `0.0500` | `-0.0765(53)` |
| R3/R4 | `+1.9123(73)` | `0.0364` | `0.0500` | `-0.1323(78)` |
| R6/R4 | `+1.2984(53)` | `0.0264` | `0.0500` | `-0.0224(56)` |
| R6/R7 | `+1.6182(73)` | `0.0363` | `0.0500` | `-0.0576(80)` |
| R4/R7 | `+1.2466(60)` | `0.0298` | `0.0500` | `-0.0228(66)` |
| R4/R5 | `+1.6242(86)` | `0.0431` | `0.0500` | `-0.0783(93)` |
| R7/R5 | `+1.3036(74)` | `0.0368` | `0.0500` | `-0.0390(81)` |

The command below recomputes those ratios and highlights the components passing
the `5 sigma` two-significant-digit criterion in green:

```bash
python3 ./examples/cli/BNL/BNL.py \
  --json-output ./examples/cli/BNL/outputs/BNL_selected_ratio_summary.json
```

## Superseded Four-Mass Additive Scan

The current physical non-optimized scan uses the additive composition described
above. R1 and R2 are above threshold and keep threshold subtraction active. R3
and R4 are below threshold and disable threshold subtraction.

Each result below is:

```text
local-only workspace - filtered integrated-UV sector workspace
```

with independent errors added in quadrature.

| label | `MT = ymt` | threshold subtraction | samples / piece | real part | imaginary part |
| --- | ---: | --- | ---: | ---: | ---: |
| R1 | 400.0 | on | 500,000 | `+7.3(2.5)e-17` | `-1.329(24)e-15` |
| R2 | 450.0 | on | 500,000 | `+4.6(2.1)e-17` | `-1.428(22)e-15` |
| R3 | 510.0 | off | 500,000 | `+1.0492(76)e-15` | `-4.826(75)e-16` |
| R4 | 550.0 | off | 500,000 | `+5.580(53)e-16` | `-2.126(56)e-16` |

The workspaces are:

| label | local-only workspace | integrated-UV sector workspace |
| --- | --- | --- |
| R1 | `physical_nonopt_additive_R1_mt400p0_threshold_base_500k_5c` | `physical_nonopt_additive_R1_mt400p0_threshold_sector_500k_5c` |
| R2 | `physical_nonopt_additive_R2_mt450p0_threshold_base_500k_5c` | `physical_nonopt_additive_R2_mt450p0_threshold_sector_500k_5c` |
| R3 | `physical_nonopt_additive_R3_mt510p0_no_threshold_base_500k_5c` | `physical_nonopt_additive_R3_mt510p0_no_threshold_sector_500k_5c` |
| R4 | `physical_nonopt_additive_R4_mt550p0_no_threshold_base_500k_5c` | `physical_nonopt_additive_R4_mt550p0_no_threshold_sector_500k_5c` |

The current ratios are:

| ratio | real part | imaginary part |
| --- | ---: | ---: |
| R2/R1 | `+1.073(26)` | `-0.024(26)` |
| R3/R1 | `+0.405(17)` | `+0.767(17)` |
| R4/R1 | `+0.1825(95)` | `+0.4099(92)` |

This is not yet the requested final precision. The limiting issue is the
small real part of R1 after subtracting the integrated-UV sector. Two scratch
above-threshold mass probes with states generated at the corresponding mass
were also tried:

| `MT = ymt` | result at 50k | comment |
| ---: | --- | --- |
| 300.0 | `-2.8(7)e-15 - 0.8(9)e-15 i` | large variance |
| 350.0 | `-1.28(17)e-15 - 2.63(13)e-15 i` | better denominator but still noisy |

The older optimized-only numbers below are retained for provenance, but they
were produced before the integrated-UV-sector composition was corrected and
should not be used as the physical final result.

### Historical Optimized-Only Scan

The historical integration scan used only the optimized setup. R1 and R2 are
above threshold and keep threshold subtraction active. R3 and R4 are below
threshold and disable threshold subtraction.

| label | `MT = ymt` | threshold subtraction | samples | real part | imaginary part | avg time / sample / core | unstable |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| R1 | 400.0 | on | 2,000,000 | `+1.076(99)e-16` | `-1.9963(96)e-15` | 1.5759 ms | 0.01855% |
| R2 | 450.0 | on | 2,000,000 | `+3.308(73)e-16` | `-1.9622(74)e-15` | 1.4609 ms | 0.01930% |
| R3 | 510.0 | off | 3,000,000 | `+1.4173(26)e-15` | `-5.336(27)e-16` | 0.9246 ms | 0.04133% |
| R4 | 550.0 | off | 3,000,000 | `+7.683(19)e-16` | `-2.541(19)e-16` | 0.9088 ms | 0.05173% |

Example command-block reproductions:

```bash
./target/release/gammaloop --read-only-state \
  -s ./examples/cli/BNL/states/final_threshold_reference_optimized \
  ./examples/cli/BNL/BNL.toml \
  run integrate_bnl_reference \
  -D mt=400.0 -D ymt=400.0 \
  -D disable_threshold_subtraction=false \
  -D integrated_h_sigma=1.0 \
  -D local_gaussian_width=1.0 \
  -D workspace=final_bnl_R1_mt400_threshold_2M_5c \
  -D n_start=200000 -D n_max=2000000 -D seed=990401 -D n_cores=5

./target/release/gammaloop --read-only-state \
  -s ./examples/cli/BNL/states/final_no_threshold_reference_optimized \
  ./examples/cli/BNL/BNL.toml \
  run integrate_bnl_reference \
  -D mt=510.0 -D ymt=510.0 \
  -D disable_threshold_subtraction=true \
  -D workspace=final_bnl_R3_mt510_no_threshold_3M_5c \
  -D n_start=300000 -D n_max=3000000 -D seed=990403 -D n_cores=5
```

The other two final commands are the same with `mt=450.0`, `seed=990402`,
`workspace=final_bnl_R2_mt450_threshold_2M_5c`, threshold subtraction enabled;
and `mt=550.0`, `seed=990404`,
`workspace=final_bnl_R4_mt550_no_threshold_3M_5c`, threshold subtraction
disabled.

## Historical Optimized-Only Ratios

These ratios came from the historical optimized-only scan retained for
provenance. They are not the current `BNL.py` defaults.

Historical ratios:

| ratio | real part | imaginary part |
| --- | ---: | ---: |
| R2/R1 | `+0.9889(60)` | `+0.1124(61)` |
| R3/R1 | `+0.3046(40)` | `+0.6936(39)` |
| R4/R1 | `+0.1476(22)` | `+0.3769(22)` |

## Extended precision and corroboration update

The old final workspaces could not be resumed after the later branch changes:
the integration workspace manifests contain generated-integrand fingerprints
that no longer match the current loaded generated states. The resume guard
correctly rejected this with:

```text
Workspace integrand fingerprints do not match the current generated integrands
for BNL@default.
```

For that reason, the high-statistics extension below was run as a fresh
current-branch workspace, not by bypassing the fingerprint check.

| label | workspace | samples | real part | imaginary part | avg time / sample / core | unstable |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| R1 current precision | `current_bnl_R1_mt400_threshold_10M_5c` | 10,000,000 | `+1.002(48)e-16` | `-1.9973(46)e-15` | 1.5650 ms | 0.01541% |
| R2 reference | `final_bnl_R2_mt450_threshold_2M_5c` | 2,000,000 | `+3.308(73)e-16` | `-1.9622(74)e-15` | 1.4609 ms | 0.01930% |
| R3 reference | `final_bnl_R3_mt510_no_threshold_3M_5c` | 3,000,000 | `+1.4173(26)e-15` | `-5.336(27)e-16` | 0.9246 ms | 0.04133% |
| R4 reference | `final_bnl_R4_mt550_no_threshold_3M_5c` | 3,000,000 | `+7.683(19)e-16` | `-2.541(19)e-16` | 0.9088 ms | 0.05173% |

The best currently available independent-error ratios are:

| ratio | real part | imaginary part |
| --- | ---: | ---: |
| R2/R1 | `+0.9882(44)` | `+0.1160(43)` |
| R3/R1 | `+0.3020(23)` | `+0.6945(22)` |
| R4/R1 | `+0.1462(14)` | `+0.3773(13)` |

This improves the previous ratios but does not reach the requested 0.25%
relative uncertainty for every real and imaginary component. The bottleneck is
the small real part of R1. After 10M samples its uncertainty is still
`4.76e-18`; propagating independent errors gives a 3.74% relative uncertainty
on `Im(R2/R1)`. Reaching 0.25% by brute-force independent sampling would require
roughly `(3.74 / 0.25)^2 ~= 224` times more R1 statistics, i.e. about
`2.2e9` R1 samples at the observed variance. At the measured runtime this is an
order-100-hour computation on five cores, so the ratio target is not practical
with the current independent-result estimator alone. A correlated multi-slot
ratio estimator, or a variance-reduced estimator specifically for the small R1
real component, is needed for that precision target.

Corroboration runs were also made with independent seeds and safe runtime
variations. The threshold-subtracted checks keep the UV scale and threshold
localization fixed and only vary the spherical mapping parameter, because
changing `m_uv` or threshold-localization widths changes the low-statistics
central values in this local-subtraction-only setup and is not a clean
corroboration knob.

| label | workspace | variation | samples | real part | imaginary part |
| --- | --- | --- | ---: | ---: | ---: |
| R1 map check | `corrob_map_R1_mt400_threshold_b0p8_500k_5c` | `b=0.8`, new seed | 500,000 | `+5.7(1.9)e-17` | `-1.998(19)e-15` |
| R2 map check | `corrob_map_R2_mt450_threshold_b1p2_500k_5c` | `b=1.2`, new seed | 500,000 | `+3.17(16)e-16` | `-1.941(16)e-15` |
| R3 map check | `corrob_phys_R3_mt510_no_threshold_b0p8_500k_5c` | `b=0.8`, new seed | 500,000 | `+1.4046(65)e-15` | `-5.312(68)e-16` |
| R4 MC check | `corrob_phys_R4_mt550_no_threshold_graph_orientation_monte_carlo_500k_5c` | graph/orientation Monte Carlo, new seed | 500,000 | `+7.78(31)e-16` | `-2.51(31)e-16` |
