# ITPhlies deployment

`instance.toml` replaces the previous server-only profile. It serves the API and
optional dashboard together on port 8080, retains the existing PostgreSQL data
path, and declares separate control/log/temp directories.

```bash
GAMMABOARD_PROFILE=release ./gb --instance ops/itphlies/instance.toml start
./gb --instance ops/itphlies/instance.toml stop --wait
```

Build dashboard assets separately in `dashboard/`. The checked-in profile is for
a trusted network; copy it to a private location to configure `[server.auth]`.
For another instance, copy the file and explicitly choose distinct ports and
state/data paths. There is no frontend port offset or nginx process.
See [deployment](../../docs/deployment.md) and [CLI guide](../../docs/local-cli.md).
