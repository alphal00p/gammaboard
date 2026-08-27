#!/usr/bin/env bash
set -euo pipefail

if (($# > 1)) || { (($# == 1)) && [[ "$1" != "--madnis" ]]; }; then
    echo "usage: scripts/test_e2e.sh [--madnis]" >&2
    exit 2
fi

cargo run -q --bin gammaboard -- db start --skip-migrations

if (($# == 1)); then
    GAMMABOARD_RUN_MADNIS_E2E=1 \
        cargo test -q --test full_stack_cli \
        full_stack_cli_gammaloop_madnis_metadata_and_batch_fuzz_e2e \
        -- --ignored --test-threads=1
else
    cargo test -q --test full_stack_cli -- --ignored \
        --test-threads="${GAMMABOARD_E2E_TEST_THREADS:-4}"
fi
