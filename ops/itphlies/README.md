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
just deploy itphlies release 1
```

From this folder (`ops/itphlies`):

```bash
just --justfile justfile deploy
just --justfile justfile deploy release 1
```

The deploy command is foreground-supervised. Stop it with `Ctrl-C`; the CLI then shuts down nginx, the backend, worker assignments, and local Postgres.
Nginx access logs are disabled in the checked-in ITPhlies deploy profile so the foreground terminal stays readable.

## Notes

- Runtime config comes from `ops/itphlies/config/runtime.toml` and is passed explicitly by the wrapper.
- The optional third argument is `port_offset`, not a literal port. Offset `1` shifts frontend/API/Postgres from `8080/4000/5400` to `8081/4001/5401` and suffixes local Postgres state paths with `-1`.
- If deploy fails with `Address already in use`, free the conflicting frontend, API, or Postgres port and retry.
- If deploy fails during DB start, inspect the Postgres log from repo root:
  ```bash
  tail -n 100 .postgres-1/logfile
  ```
