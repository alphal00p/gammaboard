#!/usr/bin/env bash
#SBATCH --job-name=gb-ctl
#SBATCH --time=00:20:00
#SBATCH --signal=B:TERM@60
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

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata/gammaboard}"
IMAGE_PATH="${WORKSPACE_ROOT}/images/gammaboard/gammaboard.sif"
SLURM_LOG_DIR="${WORKSPACE_ROOT}/logs/slurm/control"
SERVER_CFG="${WORKSPACE_ROOT}/ops/config/server.toml"
DB_PORT="5400"
FRONTEND_PORT="8080"
PORT_OFFSET="${GAMMABOARD_PORT_OFFSET:-0}"
if ! [[ "${PORT_OFFSET}" =~ ^[0-9]+$ ]]; then
  echo "GAMMABOARD_PORT_OFFSET must be a non-negative integer: ${PORT_OFFSET}" >&2
  exit 1
fi
FRONTEND_PORT="$((FRONTEND_PORT + PORT_OFFSET))"
if (( DB_PORT + PORT_OFFSET > 65535 || FRONTEND_PORT > 65535 )); then
  echo "port offset overflows frontend or Postgres port: ${PORT_OFFSET}" >&2
  exit 1
fi
DB_DATA_DIR="${WORKSPACE_ROOT}/resources/db/postgres"
DB_SOCKET_DIR="${WORKSPACE_ROOT}/resources/db/socket"

mkdir -p "${SLURM_LOG_DIR}" "${DB_SOCKET_DIR}"
mkdir -p "${DB_DATA_DIR}"
chmod 700 "${DB_DATA_DIR}"

if [[ ! -f "${IMAGE_PATH}" ]]; then
  echo "missing image: ${IMAGE_PATH}" >&2
  exit 1
fi
if [[ ! -f "${SERVER_CFG}" ]]; then
  echo "missing server config: ${SERVER_CFG}" >&2
  exit 1
fi
echo "node: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "frontend_url: http://${HOSTNAME}:${FRONTEND_PORT}"
echo "tunnel_hint_after_frontend_ready: ssh -N -L ${FRONTEND_PORT}:${HOSTNAME}:${FRONTEND_PORT} ${USER}@ubelix"

exec env -u PGTZ -u PGOPTIONS \
APPTAINERENV_LANG="${APPTAINERENV_LANG:-C.UTF-8}" \
APPTAINERENV_LC_ALL="${APPTAINERENV_LC_ALL:-C.UTF-8}" \
APPTAINERENV_TZ="${APPTAINERENV_TZ:-Etc/UTC}" \
apptainer exec -B "${WORKSPACE_ROOT}" "${IMAGE_PATH}" \
  gammaboard --resource-root "${WORKSPACE_ROOT}/resources" \
  --port-offset "${PORT_OFFSET}" \
  --postgres-public \
  deploy run --server-config "${SERVER_CFG}"
