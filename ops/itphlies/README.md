# ITPhlies Deploy

This folder contains ITPhlies-specific deploy configuration.
Shared deployment and operations notes live in [../../docs/deployment.md](../../docs/deployment.md) and [../../docs/operations.md](../../docs/operations.md).

## Files

- `config/server.toml`: backend API, frontend exposure, and deploy cleanup profile.

## Commands

From repo root:

```bash
GAMMABOARD_PROFILE=release ./gammaboard \
  deploy run \
  --server-config ops/itphlies/config/server.toml

GAMMABOARD_PROFILE=release ./gammaboard \
  --port-offset 1 \
  deploy run \
  --server-config ops/itphlies/config/server.toml
```

The deploy command is foreground-supervised. Stop it with `Ctrl-C`; the CLI then shuts down nginx, the backend, worker assignments, and local Postgres.
Nginx access logs are disabled in the checked-in ITPhlies deploy profile so the foreground terminal stays readable.

## Notes

- Runtime config uses the embedded default. Pass `--runtime-config` only for custom database/resource/Postgres settings.
- `--port-offset 1` shifts frontend/API/Postgres from `8080/4000/5400` to `8081/4001/5401` and suffixes local Postgres state paths with `-1`.
- If deploy fails with `Address already in use`, free the conflicting frontend, API, or Postgres port and retry.
- If deploy fails during DB start, inspect the Postgres log from repo root:
  ```bash
  tail -n 100 resources/db/logfile-1
  ```
