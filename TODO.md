# TODO

## Platform
- [ ] move materializer completely out of config
- [ ] implement `madnis` sampler-aggregator as a parametrization
- [ ] rework preflight checks completely
- [x] replace task-level `sample.snapshot_id` with explicit per-component source specs: `sample.sampler_aggregator` / `sample.observable` accept `"latest"` (or omitted), `{ from_name = ... }`, or `{ config = ... }`
