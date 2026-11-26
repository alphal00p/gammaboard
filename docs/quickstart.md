# Setup and examples

Run commands from the repository root on Linux with system dependencies installed:

- A current stable Rust toolchain, C/C++ build tools and pkg-config for source builds.
- Python 3.11+ for the checkout launchers and Python process workers.
- PostgreSQL server/client tools, including `initdb`, `pg_ctl` and
  `pg_stat_statements`, for a managed database.
- Native libraries for the selected backends: OpenSSL, GMP, MPFR, MPC and BLAS for
  the full build. The [development environment](../flake.nix) records the complete
  build dependencies.
- Node.js ([version](../dashboard/.nvmrc)) and npm only when building dashboard assets.

Nix is optional: `nix develop` provides a reproducible development environment.
Normal commands use installed tools on `PATH`. Current database migrations are
embedded; `sqlx-cli` is not required for instance management. External workers also
need their own evaluator/sampler dependencies.

## First run

```bash
./gb start --workers 2 --wait
./gb run create resources/templates/runs/installation-smoke.toml \
  --start --export-path "$PWD/installation-smoke.json"
./gb stop --wait
```

The unit integral should be **1.0**. The repository wrapper prepares the dashboard
and prints its URL, normally <http://localhost:4000>. For headless execution, use
`start --api-only --workers 2 --wait`. Starting services without `--workers`
launches no workers. Every explicit count launches that many additional workers.

The [CLI reference](local-cli.md) covers commands, instance selection and output
files. See [deployment](deployment.md) for release builds, executable discovery,
authentication and optional dashboard assets. HPC deployments use the
[Slurm helper](../ops/slurm/README.md).

## Dashboard setup

`./gb start` and `./gb restart` prepare the dashboard automatically. The wrapper
uses `npm ci` to install locked frontend dependencies, caches the installation,
and rebuilds assets when frontend inputs change. Node/npm are needed only for a
build. `--api-only` and `start db` skip this work.

Build output follows `server.dashboard_dir`. A custom directory containing a
prebuilt dashboard is used as supplied. Installed binaries serve prebuilt assets;
they do not run npm. See [deployment](deployment.md#build-features) for cache and
manual-build details.

## Default examples

The [dashboard catalog](../resources/templates/README.md) contains eight run cards.
These are templates, not automatically created runs. Integration-only controller
children use smaller budgets and omit plot-only tasks; their support files are
kept out of the menu.

| Run card | What it demonstrates | Build feature |
| --- | --- | --- |
| [Installation smoke](../resources/templates/runs/installation-smoke.toml) | Unit integral, no generated state or process dependency | Any build |
| [Tetrahedron native](../resources/templates/runs/tetrahedron-native.toml) | One input, three outputs, trained Havana stratification and line/PDF diagnostics | `symbolica` |
| [Tetrahedron unstratified](../resources/templates/runs/tetrahedron-unstratified.toml) | Matched ordinary Havana training and pooled inference without a partition | `symbolica` |
| [Tetrahedron process](../resources/templates/runs/tetrahedron-process.toml) | The same vector function through the Python process SDK, with a uniform proposal | Any build; Python SDK required |
| [GammaLoop four phases](../resources/templates/runs/gammaloop-four-phases.toml) | A native complex one-loop amplitude, native observables, +, −, +i, −i strata, and target-uncertainty bands | `gammaloop` |
| [Hyperparameter tuning](../resources/templates/runs/tetrahedron-tuning.toml) | Nine Voronoi-centre configurations, ranked by the tetrahedron's frozen-inference x error | `symbolica` |
| [Parameter scan](../resources/templates/runs/tetrahedron-scan.toml) | A 3×3 grid of curve sharpness/phase, including scan heatmaps | `symbolica` |
| [Tetrahedron campaign](../resources/templates/runs/tetrahedron-campaign.toml) | Two independently trained curve replicas, averaged with coefficients 1/2 and concurrent allocation | `symbolica` |

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

The tetrahedron example integrates a closed vector-valued curve over parameter time:

\[
\vec f(t)=(x(t),y(t),x(t)y(t)),\qquad t\in[0,1],
\]
\[
x(t)=\tanh(8\cos(2\pi t)),\qquad y(t)=\tanh(8\sin(2\pi t)).
\]

Saturation makes the curve spend most of its time near four vertices. The Voronoi
centres, ordered by prefixes `(0,0)`, `(0,1)`, `(1,0)`, `(1,1)`, are
`(1,1,1)`, `(1,-1,-1)`, `(-1,1,-1)`, `(-1,-1,1)`. The two discrete prefix axes
encode four stratum labels; the integrand itself has **one continuous input**.
The exact vector integral is `(0,0,0)` by half- and quarter-period symmetry.
This is an integral with respect to `dt`, without an arc-length Jacobian.

The native stratified and unstratified cards each use **1,048,576 training attempts**
and **131,072 fresh inference attempts**, 64 Havana bins, 4,096-sample training
windows, norm feedback, and seeds 10/20. They differ only in the partition and
inference estimator. The unstratified card trains its own ordinary proposal;
it is distinct from the **Pooled** comparison computed from a stratified run's draws.
Equal seeds do not imply identical draws after the proposals diverge.

Both native cards contain four tasks:

- **train:** learn the proposal; training statistics use the pooled estimator.
- **inference:** freeze the trained proposal and accumulate fresh results.
- **components-on-line:** show `x(t)`, `y(t)` and `z(t)` over the full interval.
- **pdf-line:** compare the learned PDF with the norm target, including conditional
  strata, conditional/joint/shape normalization, coverage, mismatch and value histograms.

The process card evaluates the same 1D curve with the Python SDK, using uniform
sampling and the same line diagnostics. Its `python` replacement selects an
interpreter; `script` selects the installed evaluator script. For a checkout whose
primary resource root is an instance state directory, set `script` to the absolute
path of `resources/templates/runs/support/tetrahedron.py`. The card demonstrates the
process contract, rather than the trained A/B pair.

The native GammaLoop inference task additionally provides per-stratum observable
histograms. **View** selects **Combined · stratified**, **Combined · pooled**, or
an individual **Stratum (prefix)**. **Display** offers paired estimates, paired
pooled/stratified errors and error-reduction plots. Strata do not
promise improved precision for every observable. CDF error bars remain unavailable
without cross-bin covariance.

Validate and start a selected card:

```bash
./gb local run validate resources/templates/runs/tetrahedron-native.toml --probe
./gb start --api-only --workers 6 --wait
./gb run create resources/templates/runs/tetrahedron-native.toml \
  --start --max-evaluators 5 --export-path "$PWD/tetrahedron.json"
./gb stop --wait
```

To compare the trained proposals, run these commands from the repository root.
The dashboard remains running for inspection:

```bash
./gb start --workers 6 --wait
./gb run create resources/templates/runs/tetrahedron-native.toml --start --max-evaluators 5
./gb run wait tetrahedron-native --timeout 5m
./gb run create resources/templates/runs/tetrahedron-unstratified.toml --start --max-evaluators 5
./gb run wait tetrahedron-unstratified --timeout 5m
```

Starting a run automatically waits up to 10 seconds for compatible workers;
`--timeout 0` makes one immediate attempt, and a larger timeout allows a longer
wait. `run wait` waits for successful completion by default (60-second timeout).

Six workers provide one sampler/aggregator and up to five evaluators for this
standalone run. Omitting `--max-evaluators` lets admission use all compatible free
evaluators.

`--probe` initializes configured runtimes without creating work. The GammaLoop card
needs the generated state above; the tetrahedron process card needs the SDK installed for its
chosen Python. Generated states and virtual environments are not bundled. The
campaign needs at least six compatible workers to run both replicas concurrently;
with fewer workers it runs fewer children. Its 655,360-attempt hard budget covers both children
(each has 262,144 training and 65,536 inference attempts, with no plot tasks). Tuning uses frozen-inference statistics, not adaptive training errors, and
enough training draws for several updates of each conditional grid. Its trials
share seeds to compare centre positions with less random variation. The tuning
card varies the x-coordinates of centres `(0,0)` and `(1,0)` over a 3×3 grid;
the original corners are included. The scan varies sharpness `[4,8,12]` and
periodic phase `[0,0.125,0.25]`; phase shifts preserve all three zero integrals.
The other centre coordinates, integrand,
sampler settings and per-trial budgets stay fixed. Each candidate starts its own
training and frozen-inference stages.

The [process-worker tutorials](../process_api/examples/README.md) are separate from
the dashboard catalog. The [stratification A/B benchmark](../benchmarks/stratification/README.md)
uses the same one-dimensional tetrahedron curve and calls the components directly;
it is not a distributed run or a dashboard preset.

For multiple instances, configure distinct paths and ports as described in
[deployment](deployment.md#instance-configuration).
