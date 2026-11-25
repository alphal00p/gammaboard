# Setup and examples

Run commands from the repository root. Linux x86_64 with `nix develop` is the
supported setup. Without Nix, install stable Rust, C/C++ build tools, PostgreSQL 16
server/client tools and, for optional dashboard builds, Node.js from [dashboard/.nvmrc](../dashboard/.nvmrc),
npm, and `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features postgres`).
External Python workers need Python 3.11+ and their evaluator/sampler dependencies.
Install from Git; crates.io installation is not supported.

## First run

```bash
nix develop
./gb start --workers 2 --wait
./gb run create resources/templates/runs/installation-smoke.toml \
  --start --export-path "$PWD/installation-smoke.json"
./gb stop --wait
```

The unit integral should be **1.0**. Open <http://localhost:4000> while the instance
is running and dashboard assets are installed. For headless execution, use
`start --api-only --workers 2 --wait`. Starting services without `--workers`
launches no workers. Every explicit count launches that many additional workers.

The [local CLI guide](local-cli.md) explains the consolidated `gammaboard.toml`,
explicit instance selection, output files, shutdown, and database administration.
The checkout helper defaults to `dev-optim`; use `GAMMABOARD_PROFILE=release` for
release builds. For installed binaries, build dashboard assets separately and set
`server.dashboard_dir` in the instance file. Build those assets with `npm ci` and
`npm run build` inside `dashboard/`. HPC uses the foreground service through the
[Slurm helper](../ops/slurm/README.md).

## Default examples

The [dashboard catalog](../resources/templates/README.md) contains seven run cards.
These are templates, not automatically created runs. Integration-only controller
children use smaller budgets and omit plot-only tasks; their support files are
kept out of the menu.

| Run card | What it demonstrates | Build feature |
| --- | --- | --- |
| [Installation smoke](../resources/templates/runs/installation-smoke.toml) | Unit integral, no generated state or process dependency | Any build |
| [Tetrahedron native](../resources/templates/runs/tetrahedron-native.toml) | Two inputs, three outputs, Havana stratification and the complete plot tour | `symbolica` |
| [Tetrahedron process](../resources/templates/runs/tetrahedron-process.toml) | The same vector function through the Python process SDK, with a uniform proposal | Any build; Python SDK required |
| [GammaLoop four phases](../resources/templates/runs/gammaloop-four-phases.toml) | A native complex one-loop amplitude, native observables, +, −, +i, −i strata, and target-uncertainty bands | `gammaloop` |
| [Hyperparameter tuning](../resources/templates/runs/tetrahedron-tuning.toml) | Nine Voronoi-centre configurations, ranked by the tetrahedron's frozen-inference x error | `symbolica` |
| [Parameter scan](../resources/templates/runs/tetrahedron-scan.toml) | A 3×3 grid of surface sharpness/shear, including scan heatmaps | `symbolica` |
| [Tetrahedron campaign](../resources/templates/runs/tetrahedron-campaign.toml) | Two independently trained surface replicas, averaged with coefficients 1/2 and concurrent allocation | `symbolica` |

The default build includes both features. See [build features](deployment.md#build-features)
for smaller builds; their domains, stratification and process interfaces are unchanged.

Prepare the two optional dependencies once in the worker environment:

```bash
python3 -m pip install ./process_api/python
just prepare-gammaloop resources/states/showcase/gg_hhh
```

The [preparation tool](../tools/prepare-gammaloop/README.md) uses GammaBoard's pinned GammaLoop library, refuses to overwrite an
existing state, and prepares the GL15 contribution to gg → HHH at the upstream
physical benchmark point. It includes threshold subtraction and native real/imaginary
integral, graph and amplitude-weighted external Higgs-pT observables. This is an
amplitude at fixed external kinematics, not a phase-space-integrated cross section.
The small showcase disables rotational stability probes; it is not a production
accuracy benchmark. Its short budgets can leave large fluctuations and unreliable
finite-sample error estimates for this difficult amplitude. Both real and imaginary
parts vary with the sampled loop momentum. The four Voronoi centres are exactly
`[1,0]`, `[-1,0]`, `[0,1]`, `[0,-1]`,
labelled `(0,0)`, `(0,1)`, `(1,0)`, `(1,1)` respectively.

The four-phases card includes illustrative target errors `real = 5e-7` and
`imag = 1.5e-6`. These demonstrate **Target ±1σ** bands in the mean plots and
target comparisons using both sampling and reference errors. They are not
uncertainties supplied by the upstream benchmark. Expand **real vs Target** or
**imag vs Target** to inspect the individual errors and their combined comparison
error. Set `target_error` to your reference's actual errors for scientific use.

For the surface example, define
`a = tanh(k cos(2πu))`, `b = tanh(k cos(2π(v+s u)))` and `F = (a,b,a b)`.
The default `k=6, s=0` spends most of the unit square near the four tetrahedron
corners. All three exact integrals are zero, including at the scan's nonzero shear.
The process evaluator returns the same three unweighted components. Its card's
`python` replacement selects another interpreter when needed.

The surface run's tasks form a plot tour:

- **train / inference:** allocation, hit rates, component means/errors and variance
  shares; training statistics remain pooled and inference starts fresh.
- **surface:** select x, y, z or the full three-component magnitude; use the shared
  colour scale, limits, zoom and export controls.
- **components-on-line:** all three components on an oblique line through the square.
- **pdf-plane / pdf-line:** combined/per-stratum densities, conditional/joint/shape
  normalization, mismatch, coverage, membership and optional hue/saturation encoding.
- **xy-phase:** the complex projection x+i y of the same surface, showing the phase
  colour wheel. Its real/imaginary controls refer to x/y; z is omitted. The main
  integrand remains a real three-component vector. This is not an RGB image mode.

The native GammaLoop inference task additionally provides per-stratum observable
histograms. **View** selects **Combined · stratified**, **Combined · pooled**, or
an individual **Stratum (prefix)**. **Display** offers paired estimates, paired
pooled/stratified errors and error-reduction plots. Strata do not
promise improved precision for every observable. CDF error bars remain unavailable
without cross-bin covariance.

Validate and start a selected card:

```bash
./gb local check-run resources/templates/runs/tetrahedron-native.toml --probe
./gb start --api-only --workers 6 --wait
./gb run create resources/templates/runs/tetrahedron-native.toml \
  --start --max-evaluators 4 --export-path "$PWD/tetrahedron.json"
./gb stop --wait
```

`--probe` initializes configured runtimes without creating work. The GammaLoop card
needs the generated state above; the tetrahedron process card needs the SDK installed for its
chosen Python. Generated states and virtual environments are not bundled. The
campaign needs at least six compatible workers to run both replicas concurrently;
with fewer workers it runs fewer children. Its hard budget includes training and
plots. Tuning uses frozen-inference statistics, not adaptive training errors, and
enough training draws for several updates of each conditional grid. Its trials
share seeds to compare centre positions with less random variation. The tuning
card varies the x-coordinates of centres `(0,0)` and `(1,0)` over a 3×3 grid;
the original corners are included. The other centre coordinates, integrand,
sampler settings and per-trial budgets stay fixed. Each candidate starts its own
training and frozen-inference stages.

The [process-worker tutorials](../process_api/examples/README.md) are separate from
the dashboard catalog. The [stratification A/B benchmark](../benchmarks/stratification/README.md)
uses the older one-dimensional tetrahedron path and calls the components directly;
it is not a distributed run or a dashboard preset.

For another local instance, copy `gammaboard.toml`, choose separate paths and ports,
and pass `--instance FILE` on each command. Use explicit ports for each instance. See [deployment](deployment.md) for shared machines,
authentication and cluster profiles.
