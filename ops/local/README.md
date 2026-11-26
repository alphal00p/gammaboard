# Local deployment

The checkout's `gammaboard.toml` configures a local instance. Follow the
[quickstart](../../docs/quickstart.md) to build and run it.

For another instance, copy [instance.example.toml](instance.example.toml), then
set explicit paths and distinct API/database ports. Relative paths resolve against
the copied file. Select it on each command:

```bash
gb --instance /srv/board/instance.toml start --workers 2 --wait
gb --instance /srv/board/instance.toml stop --wait
```

See [deployment](../../docs/deployment.md) for binaries, dashboard assets and access;
use the [CLI reference](../../docs/local-cli.md) for commands and maintenance.
