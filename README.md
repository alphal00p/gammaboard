# GammaBoard

GammaBoard runs distributed numerical integration with persistent runs, adaptive
samplers, replaceable workers, and a live dashboard.

## Start locally

On Linux, from a checkout with the [system dependencies](docs/quickstart.md) installed:

```bash
./gb start --workers 2 --wait
./gb run create resources/templates/runs/installation-smoke.toml \
  --start --export-path "$PWD/installation-smoke.json"
./gb stop --wait
```

`./gb start` installs frontend packages and builds the dashboard when needed, then
prints its clickable URL (normally <http://localhost:4000>). Cached assets are
reused on subsequent starts. The run should finish with an integral of **1.0**.
Use `start --api-only --workers 2 --wait` to skip frontend setup and serving.
Each worker count launches that many additional workers; startup alone launches none.

The checked-in `gammaboard.toml` consolidates local settings and explicit paths.
Use `--instance FILE` for another configuration. See the [CLI reference](docs/local-cli.md)
for separate service starts, result exports, worker restoration and database maintenance.

Anyone with dashboard access can launch configured process commands; keep the
default passwordless deployment on a trusted network.

The [default examples](docs/quickstart.md#default-examples) cover native GammaLoop
phase stratification, a 1D-to-3D tetrahedron curve with a matched unstratified
baseline, the process API, centre tuning,
scans and campaigns.

## Evaluator and sampler models

Evaluators return one **unweighted result per input point**, optionally including
that point's observable fills. Samplers generate weighted draws, adapt from ordered
feedback, and snapshot their private state. GammaBoard owns weighting, stratum
masking and statistical aggregation. Materializers and transforms connect generated
samples to integrand coordinates. The [execution model](docs/execution-model.md#ownership-and-flow)
defines these contracts for native and process implementations.

## Documentation

Start with [setup and examples](docs/quickstart.md), then use the
[documentation index](docs/README.md) to find configuration, operations, deployment
and extension guides. [Stratification](docs/stratification.md) covers partitioning,
weights, observables and uncertainty; the [execution model](docs/execution-model.md)
specifies the complete sampling/evaluation contract.

## Development

Native backends are optional: see [build features](docs/deployment.md#build-features)
for minimal, Symbolica-only and full builds. The default includes all backends.

`cargo test` runs Rust unit tests; `scripts/test_e2e.sh core` runs backend workflows
and seeded recovery checks. Set `GAMMABOARD_TEST_DATABASE_URL` to a disposable
PostgreSQL server with database-creation permission; tests create private databases.
Use `scripts/test_e2e.sh recovery 17,41` to replay fault sequences
and `scripts/test_e2e.sh physics` for configured physics integrations.

## License

MIT. Builds with the `symbolica` feature include a Symbolica application license;
no separate key is needed. Symbolica remains subject to [its license](https://symbolica.io/license.html).
