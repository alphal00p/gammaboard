# GammaBoard

GammaBoard runs distributed numerical integration with persistent runs, adaptive
samplers, replaceable workers, and a live dashboard.

## Start locally

On Linux, from a checkout:

```bash
nix develop
./gammaboard deploy
```

Open <http://localhost:8080>. In a second terminal:

```bash
nix develop
./gammaboard run create resources/templates/runs/installation-smoke.toml
./gammaboard node start-local 2
./gammaboard node auto-assign installation-smoke
```

The run should finish with an integral of **1.0**. Stop workers with
`./gammaboard node stop -a`, then press Ctrl-C in the deployment terminal.
Anyone with dashboard access can launch configured process commands; keep the
default passwordless deployment on a trusted network.

## Guides

- [Setup and examples](docs/quickstart.md)
- [Run and task configuration](docs/config.md)
- [Deployment, access, and capacity](docs/deployment.md)
- [Daily operation, performance, and recovery](docs/operations.md)
- [Benchmarks and plots](docs/benchmarking.md)
- [External evaluators and samplers](process_api/README.md)
- [UBELIX](ops/ubelix/README.md) · [ITPhlies](ops/itphlies/README.md)

## Development

`cargo test` runs Rust unit tests; `scripts/test_e2e.sh core` runs backend workflows
and seeded recovery checks. Set `GAMMABOARD_TEST_DATABASE_URL` to a disposable
PostgreSQL server with database-creation permission; tests create private databases.
Use `scripts/test_e2e.sh recovery 17,41` to replay fault sequences
and `scripts/test_e2e.sh physics` for configured physics integrations.

## License

MIT. Normal builds include a Symbolica application license; no separate key is
needed. Symbolica remains subject to [its license](https://symbolica.io/license.html).
