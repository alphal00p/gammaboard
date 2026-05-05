# ITPhlies Deploy

This folder contains ITPhlies-specific deploy configuration and wrappers.

## Files

- `config/server.toml`: backend API config for ITPhlies deploy.
- `config/deploy.toml`: deploy profile for nginx/frontend exposure and cleanup policy.
- `justfile`: thin deploy wrapper.

## Commands

From repo root:

```bash
just deploy itphlies release
just deploy itphlies release 8081
```

From this folder (`ops/itphlies`):

```bash
just --justfile justfile deploy
just --justfile justfile deploy release 8081
```

The deploy command is foreground-supervised. Stop it with `Ctrl-C`; the CLI then shuts down nginx, the backend, worker assignments, and local Postgres.
Nginx access logs are disabled in the checked-in ITPhlies deploy profile so the foreground terminal stays readable.

## Notes

- Runtime config comes from `ops/itphlies/config/runtime.toml` and is passed explicitly by the wrapper.
- The optional port argument selects the frontend/nginx port. The wrapper derives API port `PORT + 10000`, Postgres port `PORT + 20000`, database `gammaboard_PORT`, and state dirs `.postgres-PORT` / `.postgres-socket-PORT`.
- If deploy fails with `Address already in use`, free the conflicting frontend, API, or Postgres port and retry.
- If deploy fails during DB start, inspect the Postgres log from repo root:
  ```bash
  tail -n 100 .postgres-8081/logfile
  ```
