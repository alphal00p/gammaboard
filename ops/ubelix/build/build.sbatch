#!/usr/bin/env bash
#SBATCH --job-name=gb-apptainer-build
#SBATCH --time=02:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=4
#SBATCH --mem=16G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err

set -euo pipefail

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"

if [[ $# -ne 2 ]]; then
  echo "usage: build.sbatch OUTPUT.sif DEF.def" >&2
  exit 2
fi

case "$1" in
  /*) output_path="$1" ;;
  *) output_path="${WORKSPACE_ROOT}/$1" ;;
esac

case "$2" in
  /*) def_path="$2" ;;
  *) def_path="${WORKSPACE_ROOT}/$2" ;;
esac

mkdir -p "${WORKSPACE_ROOT}/logs/slurm/build" "$(dirname "${output_path}")"

cd "${WORKSPACE_ROOT}"
apptainer build --force "${output_path}" "${def_path}"
