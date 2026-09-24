# Local development

From the repository root, enter `nix develop` and run:

```sh
./gammaboard --port-offset 1 deploy --resume-workers
```

The foreground supervisor owns the API, frontend, local workers, and managed
PostgreSQL. `Ctrl-C` drains workers and stops the stack. Use the same port offset
for commands in another terminal:

```sh
./gammaboard --port-offset 1 run create resources/templates/runs/installation-smoke.toml
./gammaboard --port-offset 1 node start-local 2
./gammaboard --port-offset 1 run resume installation-smoke --max-evaluators 1
./gammaboard --port-offset 1 run wait installation-smoke --until completed
```

The checkout launcher builds the backend and refreshes the frontend for deploys.
For repeated measurements, build once and pass the binary directly to
`just benchmark`; compilation must stay outside timed intervals.

`config/runtime.toml` sets the managed PostgreSQL connection limit. Omitted
settings use embedded defaults. Changing the connection limit requires a
database restart. Use `--runtime-config PATH` for a separate configuration.
Do not start a second deployment on occupied ports.

See [examples](../../docs/examples.md), [operations](../../docs/operations.md),
and [benchmarking](../../docs/benchmarking.md).
