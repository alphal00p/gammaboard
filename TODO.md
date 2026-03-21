# TODO

## Platform
- [ ] add basic dashboard auth for steering actions
- [ ] implement `madnis` sampler-aggregator as a parametrization

## Dashboard Auth
- [ ] keep auth simple and operator-oriented: one shared admin password, no user system unless requirements change
- [ ] keep read-only endpoints open for now and gate only mutating endpoints
- [ ] add a minimal login flow backed by `GAMMABOARD_ADMIN_PASSWORD_HASH`
- [ ] keep steering APIs explicit (`pause`, `assign`, `unassign`, `append task`, `create run`) instead of generic patch endpoints
- [ ] log mutating dashboard actions through runtime log persistence
- [ ] document that this is intended for small trusted deployments behind HTTPS

## Backend Cleanup
- [ ] factor repeated node control-plane SQL in `src/stores/queries/control_plane.rs`
- [ ] add shared decode helpers in `src/stores/queries/read.rs` for id/string conversion and JSON metric decoding
- [ ] simplify `src/server/mod.rs` around common handler patterns
