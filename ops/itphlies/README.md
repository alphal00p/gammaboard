# ITPhlies Deploy

This folder contains ITPhlies-specific detached deploy configuration and wrappers.

## Files

- `config/server.toml`: backend API config for ITPhlies deploy.
- `config/deploy.toml`: detached deploy profile (nginx/frontend exposure + cleanup policy).
- `justfile`: ITPhlies deploy wrapper commands.

## Commands

From repo root:

```bash
just deploy itphlies release
just deploy-status itphlies
just stop-deploy itphlies
```

From this folder (`ops/itphlies`):

```bash
just --justfile justfile deploy
just --justfile justfile status
just --justfile justfile down
```

The wrapper runs from repo root internally so migrations, logs, and runtime paths resolve consistently.

## Notes

- Runtime config comes from `configs/runtime/default.toml` (passed explicitly by the wrapper).
- If deploy fails with `Address already in use`, free the conflicting port (typically `4000` or `5433`) and retry.
- If deploy fails during DB start, inspect the postgres log from repo root:
  ```bash
  tail -n 100 .postgres/logfile
  ```
