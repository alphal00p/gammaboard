# TODO

## Platform
- [ ] move materializer completely out of config
- [ ] implement `madnis` sampler-aggregator as a parametrization
- [ ] rework preflight checks completely
- [ ] replace task-level `sample.snapshot_id` with explicit per-component source specs: `sample.sampler_aggregator = { from_seq | config }` and `sample.observable = { from_seq | config }`, defaulting omitted fields to `from_seq = -1`
