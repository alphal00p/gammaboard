# ITPhlies deployment

[instance.toml](instance.toml) serves the API and optional dashboard on port 8080
and declares database, state, log and temporary paths.

```bash
GAMMABOARD_PROFILE=release ./gb --instance ops/itphlies/instance.toml start
./gb --instance ops/itphlies/instance.toml stop --wait
```

This profile assumes a trusted network. Copy it to a private location to configure
`[server.auth]`; adjust relative paths when copying. Concurrent instances need
distinct ports and state/data paths. See [deployment](../../docs/deployment.md)
for dashboard builds and access, and the [CLI reference](../../docs/local-cli.md)
for commands.
