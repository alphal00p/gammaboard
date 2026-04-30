#!/usr/bin/env bash
#SBATCH --job-name=gb-hello-single
#SBATCH --time=00:20:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=2
#SBATCH --mem=4G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_gratis
#SBATCH --output=logs/slurm/control/%x-%j.out
#SBATCH --error=logs/slurm/control/%x-%j.err

set -euo pipefail
umask 077

PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "${PRIVATE_ENV}" ]]; then
  # Load per-user secrets and cluster-local overrides without tracking them in git.
  source "${PRIVATE_ENV}"
fi
: "${SYMBOLICA_LICENSE:?set SYMBOLICA_LICENSE in \$HOME/.config/gammaboard/slurm.env}"
export NO_SYMBOLICA_OEM_LICENSE="${NO_SYMBOLICA_OEM_LICENSE:-1}"
export APPTAINERENV_SYMBOLICA_LICENSE="${SYMBOLICA_LICENSE}"
export APPTAINERENV_NO_SYMBOLICA_OEM_LICENSE="${NO_SYMBOLICA_OEM_LICENSE}"

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata}"
IMAGE_PATH="${WORKSPACE_ROOT}/images/gammaboard/gammaboard-latest.sif"
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime/local_postgres.template.toml"
RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
SLURM_CONTROL_LOG_DIR="${WORKSPACE_ROOT}/logs/slurm/control"
WORKER_LOG_DIR="${WORKSPACE_ROOT}/logs/slurm/workers"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-hello-single.toml"
RUN_TOML="${RUNTIME_DIR}/run-${SLURM_JOB_ID}-hello-single.toml"
INSTANCE_NAME="gb-hello-single-${SLURM_JOB_ID}"
RUN_NAME="ubelix-hello"
NODE_PREFIX="gb-hello"
DB_PORT="5433"
DEPLOY_NAME="default"
POLL_TIMEOUT_SECONDS="300"
DB_DEPLOY_DIR="${WORKSPACE_ROOT}/db/${DEPLOY_NAME}"
DB_DATA_DIR="${DB_DEPLOY_DIR}/data"
DB_SOCKET_DIR="${DB_DEPLOY_DIR}/socket"

mkdir -p "${RUNTIME_DIR}" "${SLURM_CONTROL_LOG_DIR}" "${WORKER_LOG_DIR}" "${DB_DEPLOY_DIR}" "${DB_SOCKET_DIR}"
mkdir -p "${DB_DATA_DIR}"
chmod 700 "${DB_DATA_DIR}"

if [[ ! -f "${RUNTIME_TEMPLATE}" ]]; then
  echo "missing runtime template: ${RUNTIME_TEMPLATE}" >&2
  exit 1
fi
if [[ ! -f "${IMAGE_PATH}" ]]; then
  echo "missing image: ${IMAGE_PATH}" >&2
  exit 1
fi
if ! command -v envsubst >/dev/null 2>&1; then
  echo "missing required command: envsubst" >&2
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

SAMPLER_NODE="${NODE_PREFIX}-sampler"
EVALUATOR_NODE="${NODE_PREFIX}-eval-1"

echo "node: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "runtime_config: ${RUNTIME_CONFIG}"
echo "database_host_port: 127.0.0.1:${DB_PORT}"
echo "sampler_node: ${SAMPLER_NODE}"
echo "evaluator_node: ${EVALUATOR_NODE}"

run_in_container db start

run_in_container node run --name "${SAMPLER_NODE}" >"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${SAMPLER_NODE}.out" 2>"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${SAMPLER_NODE}.err" &
SAMPLER_PID=$!
run_in_container node run --name "${EVALUATOR_NODE}" >"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${EVALUATOR_NODE}.out" 2>"${WORKER_LOG_DIR}/${SLURM_JOB_ID}-${EVALUATOR_NODE}.err" &
EVALUATOR_PID=$!

sleep 10
run_in_container node list
run_in_container run add "${RUN_TOML}"
run_in_container auto-assign "${RUN_NAME}" 1

deadline=$((SECONDS + POLL_TIMEOUT_SECONDS))
while ((SECONDS < deadline)); do
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
