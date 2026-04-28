#!/usr/bin/env bash
#SBATCH --job-name=gb-node
#SBATCH --time=00:30:00
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

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE to your .sif path}"
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:?set GAMMABOARD_WORKSPACE_ROOT to absolute workspace path}"
NODE_NAME="${NODE_NAME:?set NODE_NAME (e.g. gb-hello-sampler or gb-hello-eval-1)}"
DEPLOY_NAME="${DEPLOY_NAME:-default}"
mkdir -p "${WORKSPACE_ROOT}/logs/slurm/workers"

RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
mkdir -p "${RUNTIME_DIR}"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-${NODE_NAME}.toml"
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime/external_db_worker.toml"
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
export WORKSPACE_ROOT LOCAL_TMP DEPLOY_NAME
envsubst '${WORKSPACE_ROOT} ${LOCAL_TMP} ${DEPLOY_NAME}' \
  <"${RUNTIME_TEMPLATE}" >"${RUNTIME_CONFIG}"

db_host_port="$(awk -F' *= *' '/^url *=/{gsub(/"/,"",$2); print $2; exit}' "${RUNTIME_CONFIG}" | sed -E 's#^[^@]*@##; s#/.*$##')"

echo "node: ${HOSTNAME}"
echo "node name: ${NODE_NAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "runtime_config: ${RUNTIME_CONFIG}"
echo "database_host_port: ${db_host_port}"

env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer exec -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" \
  gammaboard --runtime-config "${RUNTIME_CONFIG}" node run --name "${NODE_NAME}"
