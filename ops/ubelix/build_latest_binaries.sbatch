#!/usr/bin/env bash
#SBATCH --job-name=gb-build-latest
#SBATCH --time=01:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=8
#SBATCH --mem=16G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=logs/%x-%j.out
#SBATCH --error=logs/%x-%j.err

set -euo pipefail

# Edit these directly on UBELIX.
WORKSPACE_PATH="/storage/research/itp_localunitaritydata"
OUT_FILE="images/gammaboard-latest.sif"
DEF_FILE="ops/gammaboard.def"
SCRATCH_BASE="/scratch/network/users/${USER}/apptainer"

TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"
OUTPUT_PATH="${WORKSPACE_PATH}/${OUT_FILE}"
DEF_PATH="${WORKSPACE_PATH}/${DEF_FILE}"

mkdir -p logs "$(dirname "${OUTPUT_PATH}")" "${TMP_BASE}" "${CACHE_BASE}"

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"

echo "host: ${HOSTNAME}"
echo "workspace_path: ${WORKSPACE_PATH}"
echo "output_path: ${OUTPUT_PATH}"
echo "def_path: ${DEF_PATH}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

apptainer build --notest "${OUTPUT_PATH}" "${DEF_PATH}"
