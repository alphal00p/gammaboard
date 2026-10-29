# Campaign concurrency — 2026-10-29

Campaigns use `max_active_runs` as a ceiling and reserve one sampler plus
`min_evaluators_per_run` evaluators per active child (default two evaluators).
Automatic assignment plans complete bundles from compatible free workers and
preserves existing operator-assigned roles. Six interchangeable workers with a
ceiling of four produce two sampler workers and four evaluator workers.

`allocation_window_samples` now counts samples per child per scheduling turn,
across all stages. Durable cumulative grants bound generation even when there is
no global sample cap. Unfinished quanta retain their slots; completed quanta are
ranked against waiting children, with pilot coverage first and incumbent priority
on equal scores. Other children retain their samplers and balanced evaluator
shares. Error convergence drains existing grants, rechecks the estimate and waits
for worker checkpoint publication before completing the campaign.

The obsolete campaign-wide allocation-start counter was removed. Campaign output
now reports requested concurrency, worker counts, resource capacity and drain
status. Auto-assignment returns an `assigned_samplers` list. These controller-state
changes use database model 7 with the existing fresh-baseline policy; sampler
checkpoint model 5, queue envelope 4 and process protocol v9 are unchanged.

## Validation

- 372 Rust library tests passed, along with the remaining non-ignored Rust suite.
  Scheduling tests cover independent quantum boundaries, stable ties, missing
  uncertainty, grants recovered without controller output, worker capability
  matching and preservation of unaffected evaluator assignments.
- All 39 core workflows passed, including recovery seeds 17 and 41 and the two
  Rust/Python process round trips. The concurrency workflow was also rerun after
  increasing child limits above the shared campaign cap.
- The concurrency workflow uses six workers and three children, with one slow
  evaluator. The third child starts while the slow child remains inside its first
  quantum, and the slow child's sampler/evaluator identities stay unchanged.
  Its sampler is then killed and restarted. The capped case finishes at exactly
  768 produced/completed samples; the error-only case also accounts for every
  grant. Each child's final RNG state matches an uninterrupted sampler with the
  same seed and accepted sample count.
- All 43 PostgreSQL store tests and six cluster-store tests passed against a fresh
  PostgreSQL 17 instance. Existing cap, stage-transition, leadership-handover and
  operator-control regressions remain covered; small fixtures explicitly request
  one evaluator where that is their intended resource policy.
- All 169 frontend tests, frontend lint and production build passed. Strict
  Clippy passed with all features and with default features disabled. Rustfmt and
  whitespace checks passed.

The optional external-state tests and seven external adapter workflows were not
run. No deployment was modified. Evidence is retained on the execution host under
`/common/dev/cedric/setup-logs/campaign-parallel-20261029/`.
