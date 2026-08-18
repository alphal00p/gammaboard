# Changelog

## Unreleased

### Breaking CLI changes

- `run create` replaces `run add`.
- `run task append` replaces `run task add`.
- `node start-local` replaces `node auto-run`.
- `node auto-assign <RUN> --max-evaluators <N>` replaces top-level
  `auto-assign <RUN> [MAX_EVALUATORS]`.
- `deploy` replaces `deploy run`.
- `auth hash-password` replaces `auth --password`; passwords are prompted
  without echo or accepted through `--password-stdin`.

### Reproducibility

- New runs persist submitted/effective TOML and build provenance.
- Python and MADNIS integration dependencies are pinned to immutable commits.
