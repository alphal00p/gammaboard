test-e2e:
    cargo test -q --test full_stack_cli -- --ignored

# Build optional runtime artifacts that are intentionally not checked in.
process-artifacts: process-rust-breit-wigner-sif symbolica-variable-theta

process-rust-breit-wigner-sif:
    cd process_api/examples/rust_breit_wigner_evaluator && apptainer build runtime.sif apptainer.def

symbolica-variable-theta:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p resources/artifacts
    "${CXX:-c++}" \
        -shared -O3 -fPIC -ffast-math -funsafe-math-optimizations \
        -o resources/artifacts/variable_theta.so \
        resources/artifacts/variable_theta.cpp

# Build optional artifacts, create every bundled example run, and start two local workers for each run.
start-example-runs: process-artifacts
    #!/usr/bin/env bash
    set -euo pipefail
    gb="${GAMMABOARD_BIN:-./gammaboard}"
    workers_per_run="${GAMMABOARD_EXAMPLE_WORKERS_PER_RUN:-2}"
    if (( workers_per_run < 2 )); then
        echo "GAMMABOARD_EXAMPLE_WORKERS_PER_RUN must be at least 2" >&2
        exit 1
    fi
    max_evaluators=$((workers_per_run - 1))
    templates=(
        resources/templates/runs/gammaloop.toml
        resources/templates/runs/ghost_bump.toml
        resources/templates/runs/hyperparameter-tuning-symbolica.toml
        resources/templates/runs/parameter-scan-symbolica.toml
        resources/templates/runs/process-evaluator-process-sampler-demo.toml
        resources/templates/runs/process-rust-apptainer-evaluator-demo.toml
        resources/templates/runs/symbolica-havana-pdf-1d2d.toml
    )
    for template in "${templates[@]}"; do
        echo "creating run from ${template}"
        run_output="$("$gb" run add "$template")"
        echo "$run_output"
        run_id="$(printf '%s\n' "$run_output" | sed -n 's/.*created run_id=\([0-9][0-9]*\).*/\1/p' | tail -1)"
        if [[ -z "$run_id" ]]; then
            echo "failed to parse run id from run-add output for ${template}" >&2
            exit 1
        fi
        "$gb" node auto-run "$workers_per_run"
        sleep "${GAMMABOARD_EXAMPLE_WORKER_REGISTER_SLEEP:-1}"
        "$gb" auto-assign "$run_id" "$max_evaluators"
    done
