#!/usr/bin/env bash
#SBATCH --job-name=gb-apptainer-build
#SBATCH --time=01:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=8
#SBATCH --mem=16G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_debug
#SBATCH --output=logs/%x-%j.out
#SBATCH --error=logs/%x-%j.err

set -euo pipefail

PROJECT_ROOT="${PROJECT_ROOT:?set PROJECT_ROOT to the absolute gammaboard repo path on UBELIX}"
OUTPUT_SIF="${OUTPUT_SIF:-${PROJECT_ROOT}/gammaboard.sif}"
BUILD_DEF="${BUILD_DEF:-${PROJECT_ROOT}/gammaboard.def}"

# UBELIX recommends using scratch-backed cache/tmp for Docker-to-SIF conversion.
SCRATCH_BASE="${SCRATCH_BASE:-/scratch/network/users/${USER}/apptainer}"
TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"

mkdir -p "${PROJECT_ROOT}/logs"
mkdir -p "${TMP_BASE}" "${CACHE_BASE}"

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"

echo "host: ${HOSTNAME}"
echo "project_root: ${PROJECT_ROOT}"
echo "build_def: ${BUILD_DEF}"
echo "output_sif: ${OUTPUT_SIF}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

cd "${PROJECT_ROOT}"
apptainer build --notest "${OUTPUT_SIF}" "${BUILD_DEF}"
