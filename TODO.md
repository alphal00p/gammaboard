# TODO

## Consistency check
- [ ] maybe add no-pw flag that allows to deploy without an admin password, could be used on ubelix as ssh tunnel is needed to access frontend
- [ ] continue docs consolidation into docs/ where it should be deployable to the gammaloop wiki
- [ ] discuss with agent what else to change before moving to version 0.2.0 by then we should be confident in api shape and process tasks

- [ ] Decide auth/no-password mode
   The README still assumes an admin password. For UBELIX behind SSH tunnel, a `no auth / trusted local deployment` mode may be simpler. If we add it, do it before `0.2.0` because it affects server config semantics.

- [ ] Reconsider Nix in first-class examples
   The README still foregrounds flake-backed process examples. Since the direction is generic process commands plus optional packaging, I’d make the primary example plain process or Apptainer, and keep Nix as “one packaging option”.

- [ ] Clean remaining complex wording
   Internally complex still exists where GammaLoop/full plots need it, but the public observable story should be “scalar/vector”. Search docs/templates for “complex” and keep only the places that are truly GammaLoop/full-plot specific.
