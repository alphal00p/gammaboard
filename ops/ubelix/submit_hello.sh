#!/usr/bin/env bash
set -euo pipefail

# Example:
#   GAMMABOARD_IMAGE=/storage/workspaces/<grp>/<ws>/gammaboard.sif \
#   GAMMABOARD_PROJECT_ROOT=/storage/workspaces/<grp>/<ws>/gammaboard \
#   GAMMABOARD_DATABASE_URL=postgresql://user:pass@dbhost:5432/gammaboard_db \
#   ./ops/ubelix/submit_hello.sh

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE}"
PROJECT_ROOT="${GAMMABOARD_PROJECT_ROOT:?set GAMMABOARD_PROJECT_ROOT}"
DATABASE_URL="${GAMMABOARD_DATABASE_URL:?set GAMMABOARD_DATABASE_URL}"

ACCOUNT="${ACCOUNT:-gratis}"
PARTITION="${PARTITION:-epyc2}"
QOS="${QOS:-job_debug}"
WORKER_TIME="${WORKER_TIME:-00:30:00}"
CONTROL_TIME="${CONTROL_TIME:-00:20:00}"
EVALUATOR_COUNT="${EVALUATOR_COUNT:-2}"
RUN_NAME="${RUN_NAME:-ubelix-hello}"
NODE_PREFIX="${NODE_PREFIX:-gb-hello}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-300}"

# Add account-specific flags here when needed, e.g.:
# EXTRA_SBATCH_ARGS="--wckey=<project>" or "--reservation=<reservation>"
EXTRA_SBATCH_ARGS="${EXTRA_SBATCH_ARGS:-}"
if [[ -n "${EXTRA_SBATCH_ARGS}" ]]; then
  # shellcheck disable=SC2206
  extra_args=( ${EXTRA_SBATCH_ARGS} )
else
  extra_args=()
fi

mkdir -p logs

common_export="ALL,GAMMABOARD_IMAGE=${IMAGE_PATH},GAMMABOARD_PROJECT_ROOT=${PROJECT_ROOT},GAMMABOARD_DATABASE_URL=${DATABASE_URL},NODE_PREFIX=${NODE_PREFIX}"

sampler_job_id="$(
  sbatch --parsable \
    --account="${ACCOUNT}" \
    --partition="${PARTITION}" \
    --qos="${QOS}" \
    --time="${WORKER_TIME}" \
    --export="${common_export},ROLE=sampler-aggregator" \
    "${extra_args[@]}" \
    ops/ubelix/slurm_node_worker.sbatch
)"

evaluator_job_id="$(
  sbatch --parsable \
    --account="${ACCOUNT}" \
    --partition="${PARTITION}" \
    --qos="${QOS}" \
    --time="${WORKER_TIME}" \
    --array="1-${EVALUATOR_COUNT}" \
    --export="${common_export},ROLE=evaluator" \
    "${extra_args[@]}" \
    ops/ubelix/slurm_node_worker.sbatch
)"

control_job_id="$(
  sbatch --parsable \
    --dependency="after:${sampler_job_id}:${evaluator_job_id}" \
    --account="${ACCOUNT}" \
    --partition="${PARTITION}" \
    --qos="${QOS}" \
    --time="${CONTROL_TIME}" \
    --export="${common_export},RUN_NAME=${RUN_NAME},EVALUATOR_COUNT=${EVALUATOR_COUNT},POLL_TIMEOUT_SECONDS=${POLL_TIMEOUT_SECONDS}" \
    "${extra_args[@]}" \
    ops/ubelix/slurm_hello_control.sbatch
)"

echo "submitted sampler job:   ${sampler_job_id}"
echo "submitted evaluator job: ${evaluator_job_id} (array 1-${EVALUATOR_COUNT})"
echo "submitted control job:   ${control_job_id}"
echo "monitor with: squeue --jobs=${sampler_job_id},${evaluator_job_id},${control_job_id}"
