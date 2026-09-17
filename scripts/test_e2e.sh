#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: scripts/test_e2e.sh [--madnis | --recovery-soak RUNS]" >&2
}

mode="${1:-all}"
if [[ "$mode" == "--recovery-soak" ]]; then
    if (($# != 2)) || ! [[ "$2" =~ ^[1-9][0-9]*$ ]]; then
        usage
        exit 2
    fi
    recovery_runs="$2"
elif [[ "$mode" == "--madnis" ]]; then
    if (($# != 1)); then
        usage
        exit 2
    fi
elif [[ "$mode" != "all" ]] || (($# != 0)); then
    usage
    exit 2
fi

cargo run -q --bin gammaboard -- db start --skip-migrations

if [[ "$mode" == "--madnis" ]]; then
    GAMMABOARD_RUN_MADNIS_E2E=1 \
        cargo test -q --test full_stack_cli \
        full_stack_cli_gammaloop_madnis_metadata_and_batch_fuzz_e2e \
        -- --ignored --test-threads=1
elif [[ "$mode" == "--recovery-soak" ]]; then
    for ((run = 1; run <= recovery_runs; run++)); do
        printf 'recovery soak %d/%d\n' "$run" "$recovery_runs"
        cargo test -q --test full_stack_cli \
            full_stack_cli_campaign_recovers_from_sampler_and_evaluator_loss \
            -- --ignored --test-threads=1
    done
    printf 'recovery soak passed: %d/%d\n' "$recovery_runs" "$recovery_runs"
else
    cargo test -q --test full_stack_cli -- --ignored \
        --test-threads="${GAMMABOARD_E2E_TEST_THREADS:-4}"
fi
