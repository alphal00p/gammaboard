#!/usr/bin/env bash
#SBATCH --job-name=gb-node
#SBATCH --time=00:30:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=2G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/%x-%A_%a.out
#SBATCH --error=logs/%x-%A_%a.err

set -euo pipefail

mkdir -p logs

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE to your .sif path}"
PROJECT_ROOT="${GAMMABOARD_PROJECT_ROOT:?set GAMMABOARD_PROJECT_ROOT to absolute project path}"
DATABASE_URL="${GAMMABOARD_DATABASE_URL:?set GAMMABOARD_DATABASE_URL to reachable Postgres URL}"
NODE_PREFIX="${NODE_PREFIX:-gb-hello}"
ROLE="${ROLE:?set ROLE to sampler-aggregator or evaluator}"

if [[ "${ROLE}" == "sampler-aggregator" ]]; then
  NODE_NAME="${NODE_PREFIX}-sampler"
elif [[ "${ROLE}" == "evaluator" ]]; then
  TASK_ID="${SLURM_ARRAY_TASK_ID:-0}"
  NODE_NAME="${NODE_PREFIX}-eval-${TASK_ID}"
else
  echo "unsupported ROLE=${ROLE}" >&2
  exit 1
fi

RUNTIME_DIR="${PROJECT_ROOT}/tmp/ubelix"
mkdir -p "${RUNTIME_DIR}"
RUNTIME_CONFIG="${RUNTIME_DIR}/runtime-${SLURM_JOB_ID}-${NODE_NAME}.toml"
trap 'rm -f "${RUNTIME_CONFIG}"' EXIT
cat >"${RUNTIME_CONFIG}" <<EOF
[database]
url = "${DATABASE_URL}"

[tracing]
persist_runtime_logs = true
db_gammaboard_level = "info"
db_external_level = "warn"

[local_postgres]
data_dir = "${TMPDIR}/.postgres"
socket_dir = "${TMPDIR}/.postgres-socket"
log_file = "${TMPDIR}/.postgres/logfile"
max_connections = 32
shared_buffers = "512MB"
effective_cache_size = "1GB"
work_mem = "16MB"
checkpoint_timeout = "30min"
max_wal_size = "1GB"
wal_compression = true
synchronous_commit = true
EOF

echo "node: ${HOSTNAME}"
echo "worker role: ${ROLE}"
echo "node name: ${NODE_NAME}"

apptainer exec -B "${PROJECT_ROOT}" "${IMAGE_PATH}" \
  gammaboard --runtime-config "${RUNTIME_CONFIG}" node run --name "${NODE_NAME}"
