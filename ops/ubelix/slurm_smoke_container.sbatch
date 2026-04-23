#!/usr/bin/env bash
#SBATCH --job-name=gb-smoke
#SBATCH --time=00:01:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=1G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/%x-%j.out
#SBATCH --error=logs/%x-%j.err

set -euo pipefail

mkdir -p logs

# Absolute path to SIF image on UBELIX filesystem.
IMAGE_PATH="${GAMMABOARD_IMAGE:-$HOME/gammaboard.sif}"

# Bind full storage paths (UBELIX scratch/workspace paths are symlinked).
BIND_PATHS="${GAMMABOARD_BIND_PATHS:-$HOME}"

if [[ ! -f "${IMAGE_PATH}" ]]; then
  echo "missing image: ${IMAGE_PATH}" >&2
  exit 1
fi

echo "node: ${HOSTNAME}"
echo "image: ${IMAGE_PATH}"
echo "bind: ${BIND_PATHS}"

apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaloop --version
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaboard --version
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaloop --help >/dev/null
apptainer exec -B "${BIND_PATHS}" "${IMAGE_PATH}" gammaboard --help >/dev/null

echo "smoke check completed"
