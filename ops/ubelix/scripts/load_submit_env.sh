#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata}"
DEFAULTS_FILE="${WORKSPACE_ROOT}/ops/config/submit_hello.env"
if [[ -f "${DEFAULTS_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${DEFAULTS_FILE}"
  set +a
fi

export WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-${WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata}}"
export IMAGE_PATH="${GAMMABOARD_IMAGE:-${IMAGE_PATH:-${WORKSPACE_ROOT}/images/gammaboard/gammaboard-latest.sif}}"
export DATABASE_URL="${GAMMABOARD_DATABASE_URL:-${DATABASE_URL:-postgresql://postgres:postgres@127.0.0.1:5433/gammaboard_db}}"
export GAMMABOARD_DATABASE_URL="${DATABASE_URL}"

export ACCOUNT="${ACCOUNT:-gratis}"
export PARTITION="${PARTITION:-epyc2}"
export QOS="${QOS:-job_debug}"

export WORKER_TIME="${WORKER_TIME:-00:15:00}"
export CONTROL_TIME="${CONTROL_TIME:-00:10:00}"
export UI_TIME="${UI_TIME:-02:00:00}"

export EVALUATOR_COUNT="${EVALUATOR_COUNT:-1}"
export RUN_NAME="${RUN_NAME:-ubelix-hello}"
export NODE_PREFIX="${NODE_PREFIX:-gb-hello}"
export POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-300}"
export DEPLOY_NAME="${DEPLOY_NAME:-default}"

export DB_PORT="${DB_PORT:-5433}"
export API_PORT="${API_PORT:-4000}"
export FRONTEND_PORT="${FRONTEND_PORT:-8080}"
export FRONTEND_BUILD_DIR="${FRONTEND_BUILD_DIR:-${WORKSPACE_ROOT}/dashboard/build}"
export LOGIN_HOST="${LOGIN_HOST:-submit03.unibe.ch}"

export EXTRA_SBATCH_ARGS="${EXTRA_SBATCH_ARGS:-}"
