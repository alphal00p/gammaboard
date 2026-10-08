# Backend tests by default; optional profiles: physics, deployment, container, all.
test-e2e *args:
    scripts/test_e2e.sh {{args}}

# A seed selects actions, their order, delays and live queue changes in both modes.
test-recovery seeds="17,41":
    scripts/test_e2e.sh recovery "{{seeds}}"

process-rust-breit-wigner-sif:
    scripts/build_process_rust_breit_wigner_sif.sh

symbolica-variable-theta:
    scripts/build_symbolica_variable_theta.sh

sync-ubelix host="ubelix" remote_folder="gammaboard":
    ops/ubelix/sync_ops.sh "{{host}}" "{{remote_folder}}"

# One CLI: measure and immediately produce separate plots and local reports.
benchmark *args:
    python3 -m benchmarks {{args}}
