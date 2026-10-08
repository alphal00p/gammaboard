#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-core}"
case "$mode" in
    core|physics|deployment|container|all) (($# <= 1)) ;;
    recovery) (($# <= 2)) ;;
    *) echo 'usage: scripts/test_e2e.sh [core | recovery [SEEDS] | physics | deployment | container | all]' >&2; exit 2 ;;
esac
if [[ "$mode" == recovery ]]; then
    export GAMMABOARD_RECOVERY_SEEDS="${2:-${GAMMABOARD_RECOVERY_SEEDS:-17,41}}"
    [[ "$GAMMABOARD_RECOVERY_SEEDS" =~ ^[0-9]+(,[0-9]+)*$ ]] || { echo 'SEEDS must be comma-separated unsigned integers' >&2; exit 2; }
fi
profile="${GAMMABOARD_TEST_PROFILE:-dev-optim}"
export GAMMABOARD_PROCESS_PYTHON="${GAMMABOARD_PROCESS_PYTHON:-python3}"
export GAMMABOARD_E2E_OUTPUT="${GAMMABOARD_E2E_OUTPUT:-$PWD/target/e2e}"
if [[ "$mode" == core || "$mode" == recovery || "$mode" == all ]]; then
    "$GAMMABOARD_PROCESS_PYTHON" -m unittest discover -s tests/fixtures
fi
# An explicitly supplied database is external to the managed local deployment.
# E2Es create and migrate private databases on this server (CREATE DATABASE needed).
if [[ -z "${GAMMABOARD_TEST_DATABASE_URL:-}" ]]; then
    cargo run --locked -q --profile "$profile" --bin gammaboard -- db start --skip-migrations
fi
run_tests() {
    cargo test --locked --profile "$profile" --test full_stack_cli "$@" \
        -- --ignored --nocapture --test-threads="${GAMMABOARD_E2E_TEST_THREADS:-2}" "${exclusions[@]}"
}
exclusions=()
case "$mode" in
    core) exclusions=(--skip adapters::); run_tests ;;
    recovery) run_tests recovery::recovery_state_machine ;;
    physics)
        exclusions=(--skip rust_apptainer --skip full_stack_deploy)
        run_tests adapters:: ;;
    deployment) run_tests adapters::full_stack_deploy ;;
    container) run_tests adapters::full_stack_cli_rust_apptainer ;;
    all) run_tests ;;
esac
if [[ "$mode" == core || "$mode" == all ]]; then
    cargo test --locked --profile "$profile" --test process_api -- --ignored --nocapture
fi
printf 'E2E evidence: %s\n' "$GAMMABOARD_E2E_OUTPUT"
