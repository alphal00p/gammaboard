# Publication TODO

GammaBoard is a research tool for physicists. Publication should optimize for a
short setup, reproducible examples, and useful error messages. It does not need
enterprise deployment or mandatory security.

Labels:

- **Codex**: repository work I can implement directly.
- **Owner**: requires a publication, licensing, or scientific-version decision.
- **Shared**: I can implement it after you choose the version or policy.

## Release Blockers

- [ ] **Owner:** Make the crate packageable on crates.io. `gammalooprs` and
  `gammaloop-api` are Git-only dependencies, which crates.io rejects even when
  they are optional. Publish those crates first or move GammaLoop support out of
  the crates.io package.
- [ ] **Owner:** Confirm that publishing GammaBoard source and crates is
  compatible with the GammaLoop and Symbolica OEM license, including
  redistribution of OEM-enabled source, crates, and binaries. Normal users do
  not need their own Symbolica license.
- [x] **Shared:** Replace Python dependency references to repository `main`
  branches with release tags or exact commits. Current integration references
  use immutable commits until a GammaBoard release tag exists.
- [x] **Codex:** Remove the tracked host-specific
  `resources/artifacts/variable_theta.so`. Keep
  `just symbolica-variable-theta` as the local build path.
- [ ] **Codex:** Verify the complete setup from a fresh clone on Linux:
  `nix develop`, `./gammaboard deploy`, create an example run, and start
  local workers.
- [ ] **Shared:** Create one tagged release and make the README installation
  commands use that version instead of `main`.

## Usability

- [x] **Codex:** Put a physicist-oriented quickstart at the top of `README.md`:
  install, launch, run one built-in integral, and inspect the result.
- [x] **Shared:** Document the minimum supported environment: Linux,
  Rust/Python/Node versions, PostgreSQL, disk requirements, and whether Nix is
  recommended or required.
- [x] **Codex:** Keep the one-line local deployment with public development
  credentials, passwordless local control, and no required TLS.
- [x] **Codex:** Keep insecure shared-machine deployment available. Print
  warnings for non-loopback dashboard access and `--postgres-public`, but never
  block startup.
- [x] **Codex:** Improve errors for common setup failures:
  missing PostgreSQL/nginx/compiler, unavailable Symbolica license, missing
  LHAPDF data, stale MadSpace builds, and absent generated artifacts.
- [x] **Codex:** Ensure every checked-in example says what it computes, what
  optional software it needs, the expected runtime scale, and the command used
  to run it.
- [x] **Codex:** Add a small, dependency-light example that always works as an
  installation smoke test.

## Reproducibility

- [x] **Shared:** Pin external scientific dependencies used by examples: GammaLoop,
  MADNIS, MadGraph7/MadSpace, PDF set names, and Python packages.
- [x] **Codex:** Record baseline run provenance: GammaBoard version/revision/features,
  submitted and effective TOML, plus operator-supplied external tool versions.
- [ ] **Codex:** Persist evaluator initialization metadata with the run provenance.
- [ ] **Codex:** Test the MadGraph examples from their documented pinned
  checkout and a user-writable LHAPDF data directory without requiring `sudo`.
- [x] **Codex:** Keep generated states, databases, virtual environments, and native
  binaries out of Git. Provide build/download commands instead.
- [ ] **Shared:** Decide which example outputs are stable enough for simple
  numerical smoke assertions.

## Packaging And Tests

- [ ] **Codex:** Run a small CI matrix on every change:
  `cargo fmt --check`, default/no-default `cargo check`, `cargo test`, frontend
  tests/build, Python package builds, and lightweight integration smoke tests.
- [ ] **Shared:** Make `cargo package` and installation from the packaged crate
  pass from a clean directory after resolving the GammaLoop crate dependency.
- [x] **Codex:** Build and inspect all Python wheels, then install them into
  clean virtual environments.
- [x] **Codex:** Add release notes describing scientific/API/config changes
  that can alter results or reproducibility.

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

1. Resolve the GammaLoop crates.io and licensing blockers.
2. Make the fresh-clone local quickstart reliable.
3. Pin scientific dependencies and validate the examples.
4. Remove generated binaries and verify package contents.
5. Add the small CI matrix.
6. Tag and test the first public release.
