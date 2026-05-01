#!/usr/bin/env bash
#SBATCH --job-name=gb-ctl
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
RUNTIME_TEMPLATE="${WORKSPACE_ROOT}/ops/config/runtime-local_postgres.template.toml"
RUNTIME_DIR="${WORKSPACE_ROOT}/runtime"
SLURM_LOG_DIR="${WORKSPACE_ROOT}/logs/slurm/control"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-ctl.toml"
INSTANCE_NAME="gb-ctl-${SLURM_JOB_ID}"
DEPLOY_CFG="${WORKSPACE_ROOT}/ops/config/deploy.toml"
DB_PORT="5433"
DEPLOY_NAME="default"
FRONTEND_PORT="8080"
DB_DEPLOY_DIR="${WORKSPACE_ROOT}/db/${DEPLOY_NAME}"
DB_DATA_DIR="${DB_DEPLOY_DIR}/data"
DB_SOCKET_DIR="${DB_DEPLOY_DIR}/socket"

mkdir -p "${RUNTIME_DIR}" "${SLURM_LOG_DIR}" "${DB_DEPLOY_DIR}" "${DB_SOCKET_DIR}"
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
if [[ ! -f "${DEPLOY_CFG}" ]]; then
  echo "missing deploy config: ${DEPLOY_CFG}" >&2
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
  apptainer instance stop "${INSTANCE_NAME}" >/dev/null 2>&1 || true
  rm -f "${RUNTIME_CONFIG}"
}
trap cleanup EXIT

apptainer instance stop "${INSTANCE_NAME}" >/dev/null 2>&1 || true
env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer instance start -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" "${INSTANCE_NAME}"

echo "node: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "runtime_config: ${RUNTIME_CONFIG}"
echo "frontend_url: http://${HOSTNAME}:${FRONTEND_PORT}"
echo "tunnel_hint_after_frontend_ready: ssh -N -L ${FRONTEND_PORT}:${HOSTNAME}:${FRONTEND_PORT} ${USER}@Ubelix"

run_in_container deploy run --deploy-config "${DEPLOY_CFG}"
