# TODO

## Consistency check
- [ ] make target spec consistent with scalar / vector aggregators
- [ ] maek full aggregators and image / pdf tasks consistent with aggregators, maybe remove vector version completely, or require selecting a component to plot (this should include the ability to plot the training weight, if we only allow plotting scalar values we can also simplify to not use the full observables at all and instead use the requires training path, which is another huge simplification)
- [ ] scan configs for unnecessary args and inconcistencies
  - [ ] the postgres socket should per default live in /tmp i think
  - [ ] persistent file paths should be resolved relative to resources, e.g. the posgres data
  - [ ] temporary files should live in the resources (per default) e.g. the samplers and possibly store paths for very large samplers in the future
- [ ] right now scattered READMES, come up with better solution for docs, that is also deployable to the gammaloop wiki
- [ ] scan frontend for dead features
- [ ] scan cli for dead features
- [ ] scan api for dead features
- [ ] make build reproducable by fixing gammaloop version in toml
- [ ] discuss with agent what else to change before moving to version 0.2.0 by then we should be confident in api shape and process tasks (we should also add a benchmark for the jsonrpc protocol just to be sure)
