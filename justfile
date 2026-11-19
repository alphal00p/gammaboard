# Backend tests by default; optional profiles: physics, deployment, container, all.
test-e2e *args:
    scripts/test_e2e.sh {{args}}

# A seed selects actions, their order, delays and live queue changes in both modes.
test-recovery seeds="17,41":
    scripts/test_e2e.sh recovery "{{seeds}}"

process-rust-evaluator-sif:
    scripts/build_process_rust_evaluator_sif.sh

# Prepare the physics state used by the GammaLoop dashboard example.
prepare-gammaloop *args:
    cargo run --locked --example prepare_gammaloop -- {{args}}

# Separate 1D path experiment; the dashboard examples use a 2D surface.
stratification-ab *args:
    cargo run --locked --profile dev-optim --no-default-features --features symbolica --example stratification_ab -- {{args}}

# One CLI: measure and immediately produce separate plots and local reports.
benchmark *args:
    python3 -m benchmarks {{args}}
