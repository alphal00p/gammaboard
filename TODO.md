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

## Performance
- [ ] batch telemetry/log writes instead of single-row inserts
- [ ] optimize completed-batch fetching so the sampler does not repeatedly rescan old rows
- [ ] implement pause by serializing the sampler-aggregator cleanly while handling the existing batch queue

## Backend Cleanup
- [ ] factor repeated node control-plane SQL in `src/stores/queries/control_plane.rs`
- [ ] add shared decode helpers in `src/stores/queries/read.rs` for id/string conversion and JSON metric decoding
- [ ] simplify `src/server/mod.rs` around common handler patterns

## Open Design
- [ ] decide whether raw log browsing should remain a custom paged resource or move onto a panel-owned selector model
- [ ] decide whether worker performance selection should stay in workspace chrome or move further into backend-owned panels
