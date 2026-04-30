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
NODE_NAME="${NODE_NAME:?set NODE_NAME (e.g. gb-hello-sampler or gb-hello-eval-1)}"
NODE_MAX_START_FAILURES="${NODE_MAX_START_FAILURES:-3}"
DATABASE_URL="${GAMMABOARD_DATABASE_URL:?set GAMMABOARD_DATABASE_URL to reachable Postgres URL}"
DEPLOY_NAME="${DEPLOY_NAME:-default}"
mkdir -p "${WORKSPACE_ROOT}/logs/slurm/workers"

RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
mkdir -p "${RUNTIME_DIR}"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-${NODE_NAME}.toml"
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime/external_db_worker.template.toml"
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
apptainer exec -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" \
  gammaboard --runtime-config "${RUNTIME_CONFIG}" node run \
    --name "${NODE_NAME}" \
    --max-start-failures "${NODE_MAX_START_FAILURES}"
