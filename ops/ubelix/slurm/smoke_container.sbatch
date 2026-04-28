#!/usr/bin/env bash
#SBATCH --job-name=gb-smoke
#SBATCH --time=00:00:30
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=1G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/build/%x-%j.out
#SBATCH --error=logs/build/%x-%j.err

set -euo pipefail

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata}"
mkdir -p "logs/build" "${WORKSPACE_ROOT}/logs/build"

# Absolute path to SIF image on UBELIX filesystem.
IMAGE_PATH="${GAMMABOARD_IMAGE:-${WORKSPACE_ROOT}/images/gammaboard/gammaboard-latest.sif}"

# Bind full storage paths (UBELIX scratch/workspace paths are symlinked).
BIND_PATHS="${GAMMABOARD_BIND_PATHS:-${WORKSPACE_ROOT}}"

require_cmd apptainer

if [[ ! -d "${WORKSPACE_ROOT}" ]]; then
  echo "missing workspace root: ${WORKSPACE_ROOT}" >&2
  exit 1
fi
if [[ ! -f "${IMAGE_PATH}" ]]; then
  echo "missing image: ${IMAGE_PATH}" >&2
  exit 1
fi
if [[ ! -e "${BIND_PATHS}" ]]; then
  echo "missing bind path: ${BIND_PATHS}" >&2
  exit 1
fi

echo "node: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "image: ${IMAGE_PATH}"
echo "bind: ${BIND_PATHS}"

# Verify dynamic linker dependencies are present inside image.
if apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" ldd /usr/local/bin/gammaloop | grep -q "not found"; then
  echo "gammaloop has missing shared library dependencies" >&2
  apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" ldd /usr/local/bin/gammaloop >&2
  exit 1
fi

# CLI probes may return non-zero in some builds; run for diagnostics but do not gate smoke on exit code.
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaloop --version || true
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaloop --help >/dev/null || true
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaboard --version || true
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaboard --help >/dev/null || true

echo "smoke check completed"
