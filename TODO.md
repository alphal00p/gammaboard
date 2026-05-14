# TODO

## Consistency check
- [ ] maybe add no-pw flag that allows to deploy without an admin password, could be used on ubelix as ssh tunnel is needed to access frontend
- [ ] right now scattered READMES, come up with better solution for docs, that is also deployable to the gammaloop wiki
- [ ] discuss with agent what else to change before moving to version 0.2.0 by then we should be confident in api shape and process tasks (we should also add a benchmark for the jsonrpc protocol just to be sure)

1. Fix flickering of toml editors in frontend

2. Add a small protocol benchmark/test
   Not for premature performance work, but to know whether stdio framing is “obviously fine”. Add one benchmark or ignored e2e that measures `eval_batch` overhead for small/medium/large batches.

3. Decide auth/no-password mode
   The README still assumes an admin password. For UBELIX behind SSH tunnel, a `no auth / trusted local deployment` mode may be simpler. If we add it, do it before `0.2.0` because it affects server config semantics.

4. Make README less encyclopedic
   The README is useful but too long for humans. I would split:
   - `README.md`: quickstart, core mental model, common commands
   - `docs/process-runtime.md`: process protocol and examples
   - `docs/config.md`: run/task/node config reference
   - `ops/*/README.md`: deployment specifics

5. Reconsider Nix in first-class examples
   The README still foregrounds flake-backed process examples. Since the direction is generic process commands plus optional packaging, I’d make the primary example plain process or Apptainer, and keep Nix as “one packaging option”.

6. Clean remaining complex wording
   Internally complex still exists where GammaLoop/full plots need it, but the public observable story should be “scalar/vector”. Search docs/templates for “complex” and keep only the places that are truly GammaLoop/full-plot specific.
