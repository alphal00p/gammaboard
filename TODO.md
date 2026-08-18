# Publication TODO

GammaBoard is a research tool for physicists. Publication should optimize for a
short setup, reproducible examples, useful errors, and dependable operation at
the intended worker scale. It does not need enterprise deployment or mandatory
security.

Labels:

- **Codex**: repository work I can implement directly.
- **Owner**: requires a publication, licensing, or scientific-version decision.
- **Shared**: I can implement it after you choose the version or policy.

## Release Blockers

- [ ] **Owner:** Make the crate packageable on crates.io. `gammalooprs` and
  `gammaloop-api` are Git-only dependencies, which crates.io rejects even when
  optional. Publish those crates first or move GammaLoop support out of the
  crates.io package.
- [ ] **Owner:** Confirm that publishing GammaBoard source and crates is
  compatible with the GammaLoop and Symbolica OEM license, including
  redistribution of OEM-enabled source, crates, and binaries.
- [ ] **Codex:** Verify the complete setup from a fresh Linux clone:
  `nix develop`, `./gammaboard deploy`, create the smoke-test run, and start
  local workers.
- [ ] **Shared:** Create one tagged release and make README installation
  commands use that version instead of `main`.
- [ ] **Codex:** Add bounded process-worker frame sizes and request deadlines;
  terminate and restart stalled process evaluators, samplers, materializers,
  and transforms.

## Scale And Reliability

- [x] **Codex:** Replace global run scans in single-run lookup, CLI name
  resolution, control-plane reconciliation, and controller child-run discovery
  with indexed, filtered queries. Add paginated run listing/API reads.
- [x] **Codex:** Bound PostgreSQL connections per node process and document
  worker-to-database capacity planning. The current per-node control and role
  pools can exceed the local 128-connection default at moderate evaluator
  counts.
- [ ] **Shared:** Define retention policy for runtime logs and evaluator/sampler
  performance history. Then add Codex-owned retention, downsampling, or
  partitioning work. Default two-second snapshots otherwise create millions of
  history rows per day on a multi-worker run.
- [x] **Codex:** Keep queue-count work bounded when failed batches accumulate;
  avoid repeatedly aggregating an unbounded historical batch set in the hot
  sampler loop.
- [x] **Codex:** Benchmark and document a supported operating envelope:
  historical run count, active evaluators, queue depth, throughput, database
  connections, and retained telemetry volume.

## Agent And CLI Operations

- [x] **Codex:** Add `--json` output for operational reads and mutations, with
  stable IDs and machine-readable errors.
- [x] **Codex:** Add explicit `run resume`, `--dry-run` where meaningful, and
  `--yes` confirmation for destructive non-interactive actions, especially
  `run remove -a`.
- [ ] **Shared:** Decide whether run creation is trusted-operator-only. Run
  TOML can launch arbitrary configured process workers, so LLMs or web users
  must not receive unrestricted create access without an approval or command
  allowlist policy.
- [x] **Codex:** Add CLI inspection commands for reproducible TOML, active
  task state, snapshots, logs, and essential run diagnostics so an LLM can
  steer runs without scraping dashboard tables.

## Reproducibility And Examples

- [x] **Codex:** Persist evaluator initialization metadata with run provenance.
- [x] **Codex:** Test MadGraph examples from their documented pinned checkout
  and a user-writable LHAPDF data directory without requiring `sudo`.
- [ ] **Shared:** Decide which example outputs are stable enough for simple
  numerical smoke assertions.

## Packaging And Tests

- [ ] **Codex:** Run a small CI matrix on every change: `cargo fmt --check`,
  default/no-default `cargo check`, `cargo test`, frontend tests/build, Python
  package builds, and lightweight integration smoke tests.
- [ ] **Shared:** Make `cargo package` and installation from the packaged crate
  pass from a clean directory after resolving the GammaLoop crate dependency.

## Explicit Non-Goals

- Mandatory authentication, TLS, private development credentials, or secure
  cookies.
- Production-grade PostgreSQL/network hardening.
- Kubernetes, cloud deployment, high-availability services, or enterprise
  observability.
- Formal governance documents or broad platform support before users need them.
- Treating warnings about intentionally insecure research deployments as fatal
  errors.

## Suggested Order

1. Add process-worker bounds and direct control-plane queries.
2. Bound per-node PostgreSQL use and choose telemetry retention.
3. Verify the fresh-clone quickstart and add the CI matrix.
4. Resolve GammaLoop crates.io and licensing blockers.
5. Make agent/CLI operation machine-safe and inspectable.
6. Tag and test the first public release.
