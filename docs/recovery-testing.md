# Backend tests and recovery evidence

Run the normal backend suite with:

```bash
just test-e2e
just test-recovery 17,41,73,101
```

These commands use real GammaBoard worker processes, the Python process SDK and
PostgreSQL. Python needs NumPy. Set `GAMMABOARD_PROCESS_PYTHON` to select it.
The default build profile is `dev-optim`; `GAMMABOARD_TEST_PROFILE=dev` reuses
ordinary development builds. The tests use Cargo's integration-test executable;
they do not recursively invoke Cargo from a running test.

If `GAMMABOARD_TEST_DATABASE_URL` is set, its server must already be running and
allow database creation. Otherwise the script starts managed local PostgreSQL.
Each scenario creates and migrates its own database. Fault injection only
affects that database and process groups created by that scenario.

`target/e2e/` contains per-scenario process logs and runtime configuration.
Recovery cases additionally save the input run, seed, action trace, invariant
observations and a database dump on failure. `recovery-summary.json` records
case outcomes, durations and the action sequences. Set `GAMMABOARD_E2E_OUTPUT`
to retain evidence elsewhere. The seed reproduces the chosen actions, delays
and tuning values; it does **not** reproduce OS or database scheduling exactly.
Keep the trace and executable/source revision when recording publication data.

## Coverage and organization

The default suite has 29 database/worker scenarios, plus the process API
roundtrip. The previous 55 ignored full-stack scenarios are consolidated into
these and seven explicitly selected adapter/deployment checks. Three ordinary
tests check configuration rejection and the recovery oracle without PostgreSQL.

| Area | What is retained |
| --- | --- |
| `tests/e2e/workflow.rs` | Installed example, training → inference, named task sources, evaluator/accumulator switching, direct Havana comparison, natural lease expiry, task isolation, telemetry export |
| `failure_policy.rs` | One shared execution matrix: sampler/build failure, materialization retry, one/two evaluator retries, terminal retry exhaustion |
| `lifecycle.rs`, `controls.rs` | Idle roles, graceful termination/restart, authenticated API including server restart, live tuning, active/tree deletion, cancelled deletion requests, launch requests |
| `search.rs`, `controllers.rs` | Shared scan/grid/random/EGO matrix, intermediate progress and allocation, result provenance, child failure, campaigns, leader handover and operator intent |
| `contracts.rs` | Definition editing/duplication, durable launch intent, atomic child creation, assignment and controller races |
| `recovery.rs` | One seeded state machine, both materialized and training modes, exact sample and feedback oracles |
| `tests/pg_store_integration.rs` | 38 focused transaction, claim-token, stale-result, migration-adjacent and resource contracts; retained at the lower layer |
| `tests/process_api.rs`, SDK unit tests | Actual Rust/Python protocol, weighted feedback, invalid responses, callback arrays, timeout/shutdown and boundary contracts |

The recovery state machine replaces three overlapping sampler checkpoint/crash
tests. Natural lease expiration and direct-library Havana equivalence stay
separate: accelerating a lease in the fuzzer cannot establish those properties.
Six failure-policy tests and eight search-controller tests share execution and
assertions instead of repeating entire worker fleets and TOML definitions.
The old recovery soak repeated one fixed campaign crash case; it is removed.
The MadNIS parameter matrix is called a boundary test, not a fuzzer.

Optional profiles are explicit and missing prerequisites fail rather than
silently reporting success:

```bash
just test-e2e physics     # Symbolica, Python, GammaLoop and MadNIS adapters
just test-e2e deployment  # port isolation; needs nginx
just test-e2e container   # builds/runs Apptainer; needs host user namespaces
just test-e2e all
```

The physics profile requires compatible generated GammaLoop states and
`GAMMABOARD_TEST_REFERENCE_STATE` from the acceptance fixture. See
[examples](examples.md) and [quickstart](quickstart.md). There are no browser
E2Es in this suite. PostgreSQL contracts still run with
`cargo test --test pg_store_integration -- --ignored` against a migrated test DB.

## Recovery fault model

Each seed processes 16,381 identifiable samples with two evaluators. The mock
evaluator has seeded variable timing, allowing out-of-order completion. Batches,
draws and training windows have different sizes, including incomplete final
batches. A first checkpoint must contain an undispatched draw and, for
training, partial feedback; otherwise the test fails its precondition.

Every case shuffles and executes all of these actions, then adds a seeded
selection of repeated actions:

- Idempotent pause/resume, with a check that the paused checkpoint stays fixed.
- Kill and replace the sampler or an evaluator using SIGKILL.
- Kill the entire worker fleet and restart fresh processes against the same DB.
- Terminate this test database's connections while workers are active.
- Change batch and generation sizes through the public live-tuning API.
- Suspend an evaluator, expire its lease, observe claim reclamation, then resume it.
- Block the checkpoint row, observe its publication INSERT waiting, kill its
  writer and abort the PostgreSQL transaction. Neither its checkpoint nor its
  companion stage may become visible.

Before an action, accepted progress must advance; a fault cannot quietly target
an idle or already completed workload. Checks read a consistent database
snapshot. After faults stop, the run must finish within a bounded wait. The
entire case has a 180-second deadline. Most crash cases accelerate lease expiry
with a test-only database update; the natural-expiry E2E retains real timers.
There are no production fault switches or new runtime tuning parameters.

## Invariants and proof obligations

Let a logical run request samples `0..N`. The test sampler produces
`x_i = (offset + i + 1) / 65536` with importance weight 2; the evaluator returns
`f(x) = x`. The expected accepted contribution is therefore exactly
`y_i = (offset + i + 1) / 32768`. These binary fractions are exactly
representable. Comparing the **whole ordered vector**, not a sum or sample
count, detects omissions, duplicates, reordering and weight errors.

| Invariant | Runtime mechanism / proof sketch | Executable evidence |
| --- | --- | --- |
| **I1. Exact accepted sequence.** Every inspected accumulator is a prefix of `(y_0,…,y_(N−1))`; at completion it is the whole sequence. | Claim tokens make retries idempotent; aggregation consumes completed batches in order. Restoring an older checkpoint restores its accumulator/cursors together and removes newer speculative work before replay. Inductively, appending the next ordered batch or restoring a consistent prefix preserves the property. | Full-vector comparison after each action and at completion; lower-layer claim/submission idempotence tests. |
| **I2. Coherent durable progress.** `0 ≤ completed ≤ produced ≤ N`; checkpoint accumulator length equals its completed cursor. Rows between checkpoint cursors contain exactly the outstanding produced samples. | Checkpoint stores sampler, generation buffer, accumulator and queue cursors together. Cleanup retains the range required by that checkpoint; recovery verifies its retained sample count before accepting it. | Consistent-snapshot queries after each action, including after cleanup/restart; checkpoint retention/restore contracts. |
| **I3. Ordered, complete feedback.** The sampler gets exactly the weighted values of its oldest complete draw, once per draw on the accepted execution path. A new training window starts only after the preceding window's feedback. No feedback is sent in non-training mode. | The generation buffer tracks original draw boundaries and accumulated feedback independently of evaluator batch sizes. The sampler returns `WAITING` at a training barrier; the runtime continues collecting feedback. Snapshot and restore include those queues and the sampler's feedback cursor. | The process sampler rejects wrong values, ordering, lengths and repeated feedback, and holds generation at training barriers; its final accepted cursor must equal `N`, with no pending draws. Negative-control tests intentionally corrupt feedback and snapshots. |
| **I4. Atomic checkpoint publication.** An aborted publication exposes neither its checkpoint nor its stage; the preceding durable checkpoint remains available. | Both writes share one PostgreSQL transaction with `synchronous_commit=on`. Recovery locks and verifies the checkpoint before changing queue/observable state. | A real blocked INSERT is terminated before commit; saved checkpoint and stage count must stay unchanged. Database transaction/statement-timeout tests cover lower-level failures. |
| **I5. Ownership and isolation.** At most one live registered sampler owns the run. Expired evaluator claims can be reclaimed; superseded claim tokens cannot overwrite accepted results. Previous-task retry rows cannot enter the next task's accumulator. | Database assignment constraints, UUID/token predicates on claims/results, and task-scoped queue queries enforce these boundaries. | Owner count throughout recovery, suspended evaluator return, stale-token SQL contracts, retained-training-history task-transition test, controller assignment/child-creation races. |
| **I6. Eventual progress after finite faults.** With a healthy DB and replacement workers, the tested run completes with no pending or claimed batches. | Workers reconnect, reclaim expired work and recover the committed checkpoint; durable queue state is authoritative and notifications are only hints. | Progress gates, final exact-vector check, empty-open-queue assertion and deadlines for every case. This is bounded experimental evidence of liveness, not an unconditional scheduling theorem. |

Accepted contributions are exactly once along the recovered logical history.
Physical evaluation and sampler callbacks **may execute again** after rollback.
Do not claim exactly-once execution or irreversible callback side effects.
Live progress beyond a checkpoint can roll back; monotonic live dashboard counts
are deliberately not an invariant.

These are proof sketches tied to implementation and falsifiable tests, not a
machine-checked proof. Their assumptions include PostgreSQL transaction/WAL
durability, deterministic restoration of the sampler's saved state, adequate
resources and finite faults. External sampler artifacts must still match their
snapshot: the test sampler is self-contained, while an overwritten external
MadNIS model is not repaired or versioned by this suite.

The suite does not simulate storage corruption, power loss, PostgreSQL crash
recovery, an indefinitely partitioned cluster, or a superseded sampler process
returning after a lease handover. SIGKILL scenarios use fail-stop samplers;
the returning stale-process scenario targets evaluators. Do not extend the
publication claim to those untested fault models.

## Finding from the new test

The database-disconnection action exposed evaluators retrying claims forever
against a dead `PgListener` connection while their node heartbeats remained
healthy. SQLx's listener repairs some failures in its receive path, but claims
execute SQL through the same connection and could repeatedly hit EOF.
GammaBoard now discards failed listener connections and resubscribes on the
next operation. The existing claim token is retained, and the two-connection
evaluator budget is unchanged. A focused PostgreSQL contract terminates the
listener during both claim and receive paths and checks subsequent claims and
notifications. The real-process state machine then checks that accepted work
and training feedback remain correct across the same fault.

## Validation on 2026-10-08

The final recovery corpus used seeds `17,41,73,101`, each with feedback off/on:
8/8 cases passed, 82 fault/control actions, 106 invariant checkpoints and 131,048
accepted samples. It took 204 seconds excluding compilation. Each completed
case had an exact final vector and no remaining open work.

The consolidated default suite passed 29/29 database/worker scenarios and the
process API roundtrip in approximately 131 seconds. Also passed: 38 PostgreSQL
contracts, 348 Rust unit tests, three Rust configuration/oracle checks, three
Python oracle controls, five physics adapters, deployment isolation, formatting
and Clippy 1.99 with warnings denied. The Apptainer profile was not rerun on this
host, which previously rejected the required unprivileged namespaces. CI is
configured to run the default backend suite and retain its evidence artifacts.

An initial shutdown test exceeded its 20-second test deadline while other suites
ran concurrently; it had accumulated 2,695 pending batches. It passed isolated
and repeated runs, and the final fixture caps generation at 1,024 samples to
avoid testing unrelated queue throughput. This evidence supports bounded
progress for the stated workloads, not a universal shutdown-latency bound.
