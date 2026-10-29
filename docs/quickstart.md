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

## Examples

Validate a card before allocating workers:

```bash
./gammaboard run validate resources/templates/runs/ghost_bump.toml
./gammaboard run validate resources/templates/runs/ghost_bump.toml --probe
```

`--probe` initializes configured runtimes, including GPUs, without creating a
run. Validate in the worker environment when paths or dependencies differ.

| Example | Requirements |
| --- | --- |
| [Installation smoke](../resources/templates/runs/installation-smoke.toml) | Two workers; no external state |
| [Havana training and images](../resources/templates/runs/ghost_bump.toml) | Built-in Symbolica evaluator |
| [GammaLoop training/inference](../resources/templates/runs/gammaloop.toml) | Generated ttH state matching the pinned GammaLoop revision |
| [GammaLoop reference integral](../resources/templates/runs/gammaloop-reference.toml) | Compatible generated state without physical selectors/observables; two unit targets |
| [Physics campaign](../resources/templates/runs/integration-campaign-qft-like.toml) | Same ttH state; independent graph-group children |
| [MadNIS](../integrations/madnis/README.md) / [MadGraph](../integrations/madgraph/README.md) | Integration-specific environment and artifacts |
| [Process examples](../process_api/README.md) | Python environment or container specified by the example |

Create a run with `./gammaboard run create CARD`, then assign workers with
`./gammaboard run resume RUN --max-evaluators N`. Run names need not be unique;
use the numeric ID when a name is ambiguous. Templates are editable in the dashboard.

Physics and large training cards can take substantial time and disk space;
check their sample counts before launching. Generated states and environments
are not included with the templates.

For another instance use `./gammaboard --port-offset 1 deploy`, and include that
same offset on subsequent commands. See [deployment](deployment.md) for shared
machines, authentication, and cluster profiles.
