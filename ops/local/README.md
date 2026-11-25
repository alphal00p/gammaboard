# Local deployment

Copy `instance.example.toml` to an explicit location and edit its paths and ports.
Relative paths resolve against that file. From the repository:

```bash
./gb start --workers 6 --wait
./gb run create resources/templates/runs/tetrahedron-native.toml \
  --start --max-evaluators 4 --export-path "$PWD/tetrahedron.json"
./gb stop --wait
```

The optional dashboard and API share the configured API port (4000 by default).
No nginx process is needed. The lightweight client can be installed separately:
`cargo install --path crates/gb`. Local bootstrap also needs the `gammaboard`
service executable; remote management only needs an endpoint and credentials.

See [CLI guide](../../docs/local-cli.md), [API contract](../../docs/management-api.md),
and [Slurm deployment](../slurm/README.md). Old runtime files remain usable for
worker execution and offline maintenance, not as a second management interface.
