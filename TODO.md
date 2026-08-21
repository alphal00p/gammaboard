# Publication TODO

GammaBoard is a research tool for physicists. Publication should optimize for a
short setup, reproducible examples, useful errors, and dependable operation at
the intended worker scale. It does not need enterprise deployment or mandatory
security.

## Release Blockers
- [ ] **Codex:** Verify the complete setup from a fresh Linux clone:
  `nix develop`, `./gammaboard deploy`, create the smoke-test run, and start
  local workers.

- [ ] **Codex:** Add bounded process-worker frame sizes and request deadlines;
  terminate and restart stalled process evaluators, samplers, materializers,
  and transforms.

## Scale And Reliability

- [ ] **Shared:** Monitor runtime logs and evaluator/sampler performance history
  during long campaigns. Add storage policy only if measured usage requires it;
  two-second snapshots can create millions of history rows per day on a
  multi-worker run.

## Agent And CLI Operations

- [ ] **Shared:** Decide whether run creation is trusted-operator-only. Run
  TOML can launch arbitrary configured process workers, so LLMs or web users
  must not receive unrestricted create access without an approval or command
  allowlist policy.

## Reproducibility And Examples

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
