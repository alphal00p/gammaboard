#!/usr/bin/env bash
#SBATCH --job-name=gb-ui-single
#SBATCH --time=02:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=2
#SBATCH --mem=4G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/control/%x-%j.out
#SBATCH --error=logs/control/%x-%j.err

set -euo pipefail

IMAGE_PATH="${GAMMABOARD_IMAGE:?set GAMMABOARD_IMAGE to your .sif path}"
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:?set GAMMABOARD_WORKSPACE_ROOT to absolute workspace path}"
SCRIPT_PATH="${WORKSPACE_ROOT}/ops/scripts/control_plane.py"

if [[ ! -f "${SCRIPT_PATH}" ]]; then
  echo "missing helper script: ${SCRIPT_PATH}" >&2
  exit 1
fi

python3 "${SCRIPT_PATH}" --mode ui-single
