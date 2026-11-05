# Setup and examples

Run commands from the repository root. Linux x86_64 with `nix develop` is the
supported setup. Without Nix, install stable Rust, C/C++ build tools, PostgreSQL 16
server/client tools, nginx, Node.js from [dashboard/.nvmrc](../dashboard/.nvmrc),
npm, and `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features postgres`).
External Python workers need Python 3.11+; MadNIS supports 3.12–3.13.
Install from Git; crates.io installation is not supported.

## First run

```bash
nix develop
./gammaboard deploy
```

The helper builds the backend and dashboard and starts PostgreSQL and nginx.
Open <http://localhost:8080>. In a second shell with the same environment:

```bash
./gammaboard run create resources/templates/runs/installation-smoke.toml
./gammaboard node start-local 2
./gammaboard node auto-assign installation-smoke
./gammaboard run wait installation-smoke --until completed
```

The unit integral should be **1.0**. Inspect it in the dashboard, then stop:

```bash
./gammaboard node stop -a
# Ctrl-C in the deployment shell.
```

The helper defaults to `dev-optim`; use `GAMMABOARD_PROFILE=release` for release
builds. `deploy --rebuild-frontend` forces a dashboard rebuild. For an installed
binary, build the dashboard separately with `npm ci && npm run build` in
`dashboard/`, then run the built `gammaboard deploy`.

## Default examples

The dashboard's default menu contains seven run cards. Integration-only controller
children use smaller budgets and omit plot-only tasks; their support files are
kept out of the menu.

| Run card | What it demonstrates |
| --- | --- |
| [Installation smoke](../resources/templates/runs/installation-smoke.toml) | Unit integral, no generated state or process dependency |
| [Tetrahedron surface](../resources/templates/runs/tetrahedron-surface.toml) | Two inputs, three outputs, Havana stratification and the complete plot tour |
| [Tetrahedron process](../resources/templates/runs/tetrahedron-process.toml) | The same vector function through the Python process SDK, with a uniform proposal |
| [GammaLoop four phases](../resources/templates/runs/gammaloop-stratified.toml) | A native complex one-loop amplitude, native observables and +, −, +i, −i strata |
| [Hyperparameter tuning](../resources/templates/runs/hyperparameter-tuning.toml) | Nine Voronoi-centre configurations, ranked by the tetrahedron's frozen-inference x error |
| [Parameter scan](../resources/templates/runs/parameter-scan.toml) | A 3×3 grid of surface sharpness/shear, including scan heatmaps |
| [Integration campaign](../resources/templates/runs/integration-campaign.toml) | Two independent GammaLoop replicas, averaged with coefficients 1/2 and concurrent allocation |

Prepare the two optional dependencies once in the worker environment:

```bash
python3 -m pip install ./process_api/python
cargo run --example prepare_showcase -- resources/states/showcase/gg_hhh
```

The generator uses GammaBoard's pinned GammaLoop library, refuses to overwrite an
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
histograms, paired basic/stratified errors and error-reduction plots. Strata do not
promise improved precision for every observable. CDF error bars remain unavailable
without cross-bin covariance.

Validate, create and assign a selected card:

```bash
./gammaboard run validate resources/templates/runs/tetrahedron-surface.toml --probe
./gammaboard run create resources/templates/runs/tetrahedron-surface.toml
./gammaboard node start-local 6
./gammaboard run resume tetrahedron-surface --max-evaluators 4
```

`--probe` initializes configured runtimes without creating work. The native card
needs the generated state above; the process card needs the SDK installed for its
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

Specialist examples live with their integrations instead of in the default menu:
[MadNIS](../integrations/madnis/README.md), [MadGraph](../integrations/madgraph/README.md),
[process API](../process_api/README.md), [compiled Symbolica](../examples/symbolica-variable-theta/run.toml),
and the [GammaLoop map reference](../examples/gammaloop/reference.toml).

For another instance use `./gammaboard --port-offset 1 deploy` and the same offset
on subsequent CLI commands. See [deployment](deployment.md) for shared machines,
authentication and cluster profiles.
