#!/usr/bin/env bash
#SBATCH --job-name=gb-smk
#SBATCH --time=00:00:30
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=1G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_gratis
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err

set -euo pipefail
PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "$PRIVATE_ENV" ]]; then source "$PRIVATE_ENV"; fi
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"
exec python3 "${WORKSPACE_ROOT}/ops/build/verify-image.py" "$WORKSPACE_ROOT" "${GAMMABOARD_IMAGE_MANIFEST:-$WORKSPACE_ROOT/images/gammaboard/current.json}"
