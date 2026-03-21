# TODO

## Platform
- [ ] implement `madnis` sampler-aggregator as a parametrization

## Dashboard Auth
- [ ] keep steering APIs explicit (`pause`, `assign`, `unassign`, `append task`, `create run`) instead of generic patch endpoints

## Backend Cleanup
- [ ] factor repeated node control-plane SQL in `src/stores/queries/control_plane.rs`
- [ ] simplify `src/server/mod.rs` around common handler patterns
