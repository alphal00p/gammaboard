# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the
shared control plane. The dashboard shows runs, task output, nodes, performance,
logs, and active runtime settings.

## Start Here

Use the self-contained docs quickstart for setup and operator workflows:

- [docs/quickstart.md](docs/quickstart.md)
- [docs/README.md](docs/README.md)

Minimal local launch from the repo root:

```bash
./gammaboard deploy run
```

Minimal ITPhlies release launch:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  deploy run \
  --server-config ops/itphlies/config/server.toml
```

For UBELIX Slurm/Apptainer operation, use [ops/ubelix/README.md](ops/ubelix/README.md).

If you use Nix, the repository flake provides a development/operator shell with
Rust, Node.js, PostgreSQL, nginx, Apptainer, and helper tools. It is not the
deployment mechanism; production runs use the `gammaboard` binary, ops configs,
and, on UBELIX, Apptainer/Slurm images.

## Core Commands

- `gammaboard deploy run`: supervise local Postgres, backend API, and nginx/frontend in one foreground process.
- `gammaboard run`: create, list, pause, clone, remove, and append tasks to runs.
- `gammaboard node`: run workers, list nodes, assign roles, unassign, and request shutdown.
- `gammaboard db`: manage the local PostgreSQL instance used by the active runtime config.
- `gammaboard server`: run only the backend API for API-only/manual setups.

The repo-root `./gammaboard` helper builds the current CLI before forwarding
arguments to it. It uses `dev-optim` by default, `GAMMABOARD_PROFILE=release`
for release builds, and also builds the dashboard frontend for `deploy run`.

## Model

- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- The node supervisor leader advances task state and controller tasks in the control plane; sampler/evaluator assignments are reserved for compute work.
- Run names are human-facing and not unique. Ambiguous CLI name references fail.
- Node identity is a human-facing `name` plus a live-process `uuid`.
- Process evaluators and samplers are external commands speaking `gammaboard-jsonrpc-v1`.

## Process API

Custom evaluators and samplers can run as ordinary child processes. GammaBoard
sends framed JSON-RPC requests on stdin and reads responses from stdout; stderr
is used for logs. See [docs/process-runtime.md](docs/process-runtime.md) for
the full protocol.

For Python runtimes, install the small helper package:

```bash
pip install "gammaboard-process @ git+https://github.com/alphal00p/gammaboard.git@main#subdirectory=process_api/python"
```

Minimal evaluator:

```python
from gammaboard_process import Evaluator, run_evaluator

class MyEvaluator(Evaluator):  # ABC inheritance is optional.
    def eval(self, xs_discrete, xs_continuous):
        return xs_continuous[:, 0]

run_evaluator(MyEvaluator)
```

Minimal sampler:

```python
from gammaboard_process import SampleBatch, Sampler, run_sampler

class MySampler(Sampler):  # ABC inheritance is optional.
    def sample_plan(self):
        return {"kind": "produce", "nr_samples": 1024}

    def produce_latent_batch(self, nr_samples):
        return SampleBatch(xs_discrete, xs_continuous, weights)

    def ingest_training_values(self, training_values):
        pass

    def snapshot(self):
        return {}

run_sampler(MySampler)
```

GammaBoard derives `discrete_cardinalities` and `continuous_dims` from the run
domain and passes them as keyword arguments together with the process config
`args`. Sampler restore may additionally implement
`from_snapshot(snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)`.
Examples live in
[process_api/examples](process_api/examples).

## Development

```bash
cargo test -q
just test-e2e
```

Build without the heavy GammaLoop dependency:

```bash
cargo build --no-default-features
```

## License

Gammaboard is intended to be distributed under the MIT License.

GammaLoop support depends on GammaLoop and Symbolica. GammaLoop has no usage
restrictions, but Symbolica has its own license terms:
https://symbolica.io/license.html

Builds without the default `gammaloop` feature do not link GammaLoop, but
Gammaboard still depends directly on Symbolica for built-in Symbolica
evaluators.
