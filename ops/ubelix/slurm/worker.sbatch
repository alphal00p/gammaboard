#!/usr/bin/env bash
#SBATCH --job-name=gb-wrk
#SBATCH --time=00:30:00
#SBATCH --signal=B:TERM@60
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=2G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_gratis
#SBATCH --output=logs/slurm/workers/%x-%A_%a.out
#SBATCH --error=logs/slurm/workers/%x-%A_%a.err

set -euo pipefail

PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "${PRIVATE_ENV}" ]]; then
  # Load per-user secrets and cluster-local overrides without tracking them in git.
  source "${PRIVATE_ENV}"
fi
: "${SYMBOLICA_LICENSE:?set SYMBOLICA_LICENSE in \$HOME/.config/gammaboard/slurm.env}"
export NO_SYMBOLICA_OEM_LICENSE="${NO_SYMBOLICA_OEM_LICENSE:-1}"
export APPTAINERENV_SYMBOLICA_LICENSE="${SYMBOLICA_LICENSE}"
export APPTAINERENV_NO_SYMBOLICA_OEM_LICENSE="${NO_SYMBOLICA_OEM_LICENSE}"

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE to your .sif path}"
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:?set GAMMABOARD_WORKSPACE_ROOT to absolute workspace path}"
NODE_NAME="${NODE_NAME:?set NODE_NAME (e.g. w-1)}"
NODE_MAX_START_FAILURES="${NODE_MAX_START_FAILURES:-3}"
DATABASE_URL="${GAMMABOARD_DATABASE_URL:?set GAMMABOARD_DATABASE_URL to reachable Postgres URL}"
DEPLOY_NAME="${DEPLOY_NAME:-default}"
CONTROL_JOB_ID="${CONTROL_JOB_ID:-}"
mkdir -p "${WORKSPACE_ROOT}/logs/slurm/workers"

RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
mkdir -p "${RUNTIME_DIR}"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-${NODE_NAME}.toml"
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime.template.toml"

worker_pid=""
monitor_pid=""

stop_worker() {
  set +e
  trap - TERM INT EXIT
  if [[ -n "${monitor_pid}" ]]; then
    kill "${monitor_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${worker_pid}" ]] && kill -0 "${worker_pid}" >/dev/null 2>&1; then
    kill -TERM "${worker_pid}" >/dev/null 2>&1 || true
    for _ in $(seq 1 55); do
      if ! kill -0 "${worker_pid}" >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done
    if kill -0 "${worker_pid}" >/dev/null 2>&1; then
      echo "worker ${NODE_NAME} did not exit before grace timeout; forcing termination" >&2
      kill -KILL "${worker_pid}" >/dev/null 2>&1 || true
    fi
    wait "${worker_pid}" >/dev/null 2>&1 || true
  fi
  rm -f "${RUNTIME_CONFIG}"
}

handle_term() {
  echo "received termination signal; stopping worker ${NODE_NAME}" >&2
  stop_worker
  exit 143
}

trap handle_term TERM INT
trap 'rm -f "${RUNTIME_CONFIG}"' EXIT
if [[ ! -f "${RUNTIME_TEMPLATE}" ]]; then
  echo "missing runtime template: ${RUNTIME_TEMPLATE}" >&2
  exit 1
fi
if ! command -v envsubst >/dev/null 2>&1; then
  echo "missing required command: envsubst (install gettext)" >&2
  exit 1
fi
LOCAL_TMP="${SLURM_TMPDIR:-${TMPDIR:-/tmp}}"
export DATABASE_URL WORKSPACE_ROOT LOCAL_TMP DEPLOY_NAME
envsubst '${DATABASE_URL} ${WORKSPACE_ROOT} ${LOCAL_TMP} ${DEPLOY_NAME}' \
  <"${RUNTIME_TEMPLATE}" >"${RUNTIME_CONFIG}"

db_url="$(awk -F' *= *' '/^url *=/{gsub(/"/,"",$2); print $2; exit}' "${RUNTIME_CONFIG}")"
if [[ -z "${db_url}" ]]; then
  echo "missing [database].url in runtime config: ${RUNTIME_CONFIG}" >&2
  exit 1
fi
db_host_port="$(echo "${db_url}" | sed -E 's#^[^@]*@##; s#/.*$##')"
db_host="${db_host_port%%:*}"

if [[ "${db_host}" == "127.0.0.1" || "${db_host}" == "localhost" || "${db_host}" == "::1" ]]; then
  echo "database.url points to loopback (${db_host_port}); separate Slurm worker jobs need the control-node hostname/IP" >&2
  echo "set GAMMABOARD_DATABASE_URL to the control-node DB listener before submitting this worker" >&2
  exit 1
fi

echo "node: ${HOSTNAME}"
echo "node name: ${NODE_NAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "runtime_config: ${RUNTIME_CONFIG}"
echo "database_host_port: ${db_host_port}"
echo "max_start_failures: ${NODE_MAX_START_FAILURES}"
echo "node_capabilities: ${NODE_CAPABILITIES:-}"
if [[ -n "${CONTROL_JOB_ID}" ]]; then
  echo "control_job_id: ${CONTROL_JOB_ID}"
fi

node_capability_args=()
if [[ -n "${NODE_CAPABILITIES:-}" ]]; then
  read -r -a node_capability_args <<< "${NODE_CAPABILITIES}"
fi
apptainer_gpu_args=()
if [[ " ${NODE_CAPABILITIES:-} " =~ (^|[[:space:]])--capability[[:space:]]+gpu=([1-9][0-9]*)([[:space:]]|$) ]]; then
  apptainer_gpu_args+=(--nv)
fi
echo "apptainer_gpu_args: ${apptainer_gpu_args[*]:-}"

# Fail fast if DB is not reachable from this worker allocation.
env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer exec -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" \
  psql "${db_url}" -c "SELECT 1;" >/dev/null

env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer exec "${apptainer_gpu_args[@]}" -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" \
  gammaboard --runtime-config "${RUNTIME_CONFIG}" node run \
    --name "${NODE_NAME}" \
    --max-start-failures "${NODE_MAX_START_FAILURES}" \
    "${node_capability_args[@]}" &
worker_pid="$!"

if [[ -n "${CONTROL_JOB_ID}" ]]; then
  (
    while kill -0 "${worker_pid}" >/dev/null 2>&1; do
      if [[ -z "$(squeue -h -j "${CONTROL_JOB_ID}" -o "%i")" ]]; then
        echo "control job ${CONTROL_JOB_ID} is no longer active; stopping worker ${NODE_NAME}" >&2
        kill -TERM "${worker_pid}" >/dev/null 2>&1 || true
        for _ in $(seq 1 55); do
          if ! kill -0 "${worker_pid}" >/dev/null 2>&1; then
            exit 0
          fi
          sleep 1
        done
        echo "worker ${NODE_NAME} did not exit after control job ended; forcing termination" >&2
        kill -KILL "${worker_pid}" >/dev/null 2>&1 || true
        exit 0
      fi
      sleep 5
    done
  ) &
  monitor_pid="$!"
else
  monitor_pid=""
fi

set +e
wait "${worker_pid}"
status="$?"
if [[ -n "${monitor_pid}" ]]; then
  kill "${monitor_pid}" >/dev/null 2>&1 || true
fi
rm -f "${RUNTIME_CONFIG}"
exit "${status}"
