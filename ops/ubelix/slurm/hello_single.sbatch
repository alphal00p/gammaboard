#!/usr/bin/env bash
#SBATCH --job-name=gb-hello-single
#SBATCH --time=00:20:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=2
#SBATCH --mem=4G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/control/%x-%j.out
#SBATCH --error=logs/control/%x-%j.err

set -euo pipefail

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE to your .sif path}"
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:?set GAMMABOARD_WORKSPACE_ROOT to absolute workspace path}"
RUN_NAME="${RUN_NAME:-ubelix-hello}"
NODE_PREFIX="${NODE_PREFIX:-gb-hello}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-300}"
DEPLOY_NAME="${DEPLOY_NAME:-default}"
DB_PORT="${DB_PORT:-5433}"
CONTROL_LOG_DIR="${WORKSPACE_ROOT}/logs/control"
WORKER_LOG_DIR="${WORKSPACE_ROOT}/logs/workers"
INSTANCE_NAME="gb-hello-single-${SLURM_JOB_ID}"

mkdir -p "${CONTROL_LOG_DIR}" "${WORKER_LOG_DIR}"

RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
mkdir -p "${RUNTIME_DIR}"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-hello-single.toml"
RUN_TOML="${RUNTIME_DIR}/run-${SLURM_JOB_ID}-hello-single.toml"
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime_local_postgres.template.toml"
DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:${DB_PORT}/gammaboard_db"

if [[ ! -f "${RUNTIME_TEMPLATE}" ]]; then
  echo "missing runtime template: ${RUNTIME_TEMPLATE}" >&2
  exit 1
fi
if ! command -v envsubst >/dev/null 2>&1; then
  echo "missing required command: envsubst (install gettext)" >&2
  exit 1
fi

LOCAL_TMP="${SLURM_TMPDIR:-${TMPDIR:-/tmp}}"
export WORKSPACE_ROOT LOCAL_TMP DEPLOY_NAME DB_PORT
envsubst '${WORKSPACE_ROOT} ${LOCAL_TMP} ${DEPLOY_NAME} ${DB_PORT}' \
  <"${RUNTIME_TEMPLATE}" >"${RUNTIME_CONFIG}"

cat >"${RUN_TOML}" <<EOF
name = "${RUN_NAME}"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 5_000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
EOF

SAMPLER_NODE="${NODE_PREFIX}-sampler"
EVALUATOR_NODE="${NODE_PREFIX}-eval-1"

run_in_container() {
  env -u PGTZ -u PGOPTIONS \
  APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
  APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
  APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
  apptainer exec "instance://${INSTANCE_NAME}" \
    gammaboard --runtime-config "${RUNTIME_CONFIG}" "$@"
}

cleanup() {
  set +e
  run_in_container node stop --all
  run_in_container run pause "${RUN_NAME}"
  run_in_container db stop
  if [[ -n "${SAMPLER_PID:-}" ]]; then
    kill "${SAMPLER_PID}" 2>/dev/null || true
    wait "${SAMPLER_PID}" 2>/dev/null || true
  fi
  if [[ -n "${EVALUATOR_PID:-}" ]]; then
    kill "${EVALUATOR_PID}" 2>/dev/null || true
    wait "${EVALUATOR_PID}" 2>/dev/null || true
  fi
  apptainer instance stop "${INSTANCE_NAME}" 2>/dev/null || true
  rm -f "${RUNTIME_CONFIG}" "${RUN_TOML}"
}
trap cleanup EXIT

apptainer instance stop "${INSTANCE_NAME}" 2>/dev/null || true
env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer instance start -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" "${INSTANCE_NAME}"

db_host_port="$(echo "${DATABASE_URL}" | sed -E 's#^[^@]*@##; s#/.*$##')"
echo "node: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "runtime_config: ${RUNTIME_CONFIG}"
echo "database_host_port: ${db_host_port}"
echo "sampler_node: ${SAMPLER_NODE}"
echo "evaluator_node: ${EVALUATOR_NODE}"

echo "starting local postgres"
run_in_container db start

run_in_container node run --name "${SAMPLER_NODE}" >"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${SAMPLER_NODE}.out" 2>"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${SAMPLER_NODE}.err" &
SAMPLER_PID=$!
run_in_container node run --name "${EVALUATOR_NODE}" >"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${EVALUATOR_NODE}.out" 2>"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${EVALUATOR_NODE}.err" &
EVALUATOR_PID=$!

echo "waiting for workers to announce"
sleep 10
run_in_container node list

echo "creating run ${RUN_NAME}"
run_in_container run add "${RUN_TOML}"

echo "auto-assigning workers"
run_in_container auto-assign "${RUN_NAME}" 1

deadline=$((SECONDS + POLL_TIMEOUT_SECONDS))
while ((SECONDS < deadline)); do
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) poll: node list"
  run_in_container node list
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) poll: task list"
  TASK_OUTPUT="$(run_in_container run task list "${RUN_NAME}" || true)"
  echo "${TASK_OUTPUT}"
  if grep -q "state=completed" <<<"${TASK_OUTPUT}"; then
    echo "task completed"
    break
  fi
  if grep -q "state=failed" <<<"${TASK_OUTPUT}"; then
    echo "task failed" >&2
    exit 1
  fi
  sleep 5
done

if ((SECONDS >= deadline)); then
  echo "timed out waiting for run completion (${POLL_TIMEOUT_SECONDS}s)" >&2
  exit 1
fi

run_in_container run list "${RUN_NAME}"
echo "hello-single finished"
