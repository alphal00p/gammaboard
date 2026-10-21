#!/usr/bin/env bash
#SBATCH --job-name=gb-build
#SBATCH --time=00:45:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=6
#SBATCH --mem=8G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err

set -euo pipefail
PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "$PRIVATE_ENV" ]]; then source "$PRIVATE_ENV"; fi
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"
exec bash "${WORKSPACE_ROOT}/ops/build/service-build.sh" gammaboard
