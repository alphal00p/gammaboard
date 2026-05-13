#!/usr/bin/env bash
#SBATCH --job-name=gb-py-build
#SBATCH --time=02:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=4
#SBATCH --mem=16G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=/storage/research/itp_localunitaritydata/gammaboard/logs/slurm/build/%x-%j.out
#SBATCH --error=/storage/research/itp_localunitaritydata/gammaboard/logs/slurm/build/%x-%j.err

set -euo pipefail

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata/gammaboard}"
PYTHON_DEF_REL="ops/build/python.def"
PYTHON_RUNTIME_REL="${PYTHON_RUNTIME_REL:-resources/runtimes/madnis_gammaboard_api}"
SCRATCH_BASE="${SCRATCH_BASE:-/scratch/network/users/${USER}/apptainer}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

write_meta() {
  local meta_path="$1"
  shift
  : >"${meta_path}"
  for line in "$@"; do
    printf '%s\n' "${line}" >>"${meta_path}"
  done
  printf 'built_at_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${meta_path}"
}

require_cmd apptainer
require_cmd stat

TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"
PYTHON_DEF_PATH="${WORKSPACE_ROOT}/${PYTHON_DEF_REL}"
PYTHON_DEF_DIR="$(dirname "${PYTHON_DEF_PATH}")"
PYTHON_DEF_FILE_NAME="$(basename "${PYTHON_DEF_PATH}")"
PYTHON_RUNTIME_DIR="${WORKSPACE_ROOT}/${PYTHON_RUNTIME_REL}"
PYTHON_OUT_IMAGE="${PYTHON_RUNTIME_DIR}/runtime.sif"
PYTHON_TMP_IMAGE="${PYTHON_RUNTIME_DIR}/.runtime.tmp-${SLURM_JOB_ID:-$$}.sif"
PYTHON_META_FILE="${PYTHON_RUNTIME_DIR}/runtime.meta"

mkdir -p "${WORKSPACE_ROOT}/logs/slurm/build" "${TMP_BASE}" "${CACHE_BASE}" "${PYTHON_RUNTIME_DIR}"

if [[ ! -f "${PYTHON_DEF_PATH}" ]]; then
  echo "missing def file: ${PYTHON_DEF_PATH}" >&2
  exit 1
fi
if [[ ! -f "${PYTHON_RUNTIME_DIR}/pyproject.toml" ]]; then
  echo "missing python runtime project: ${PYTHON_RUNTIME_DIR}/pyproject.toml" >&2
  exit 1
fi

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"
export PYTHON_PROJECT_DIR="/workspace/${PYTHON_RUNTIME_REL}"
export APPTAINERENV_PYTHON_PROJECT_DIR="${PYTHON_PROJECT_DIR}"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "python_def: ${PYTHON_DEF_PATH}"
echo "python_runtime_project: ${PYTHON_RUNTIME_DIR}"
echo "python_project_dir_in_build: ${PYTHON_PROJECT_DIR}"
echo "python_image: ${PYTHON_OUT_IMAGE}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

(
  cd "${PYTHON_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${PYTHON_TMP_IMAGE}" "${PYTHON_DEF_FILE_NAME}"
)
mv -f "${PYTHON_TMP_IMAGE}" "${PYTHON_OUT_IMAGE}"

PYTHON_IMAGE_SIZE_BYTES="$(stat -c '%s' "${PYTHON_OUT_IMAGE}")"
if command -v sha256sum >/dev/null 2>&1; then
  PYTHON_IMAGE_SHA256="$(sha256sum "${PYTHON_OUT_IMAGE}" | awk '{print $1}')"
else
  PYTHON_IMAGE_SHA256="-"
fi

write_meta "${PYTHON_META_FILE}" \
kind=python-runtime \
def_path="${PYTHON_DEF_REL}" \
runtime_project="${PYTHON_RUNTIME_REL}" \
slurm_job_id="${SLURM_JOB_ID:-}" \
build_host="${HOSTNAME}" \
image_size_bytes="${PYTHON_IMAGE_SIZE_BYTES}" \
image_sha256="${PYTHON_IMAGE_SHA256}"
