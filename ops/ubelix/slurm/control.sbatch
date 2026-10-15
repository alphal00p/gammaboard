#!/usr/bin/env bash
#SBATCH --job-name=gb-ctl
#SBATCH --time=00:20:00
#SBATCH --signal=B:TERM@60
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=2
#SBATCH --mem=4G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_gratis
#SBATCH --output=logs/slurm/control/%x-%j.out
#SBATCH --error=logs/slurm/control/%x-%j.err

set -euo pipefail
umask 077
PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "$PRIVATE_ENV" ]]; then source "$PRIVATE_ENV"; fi
MANIFEST="${GAMMABOARD_MANIFEST:?deployment recipe required}"
exec python3 "$(dirname "$MANIFEST")/launcher/ubelix.py" run-control --manifest "$MANIFEST"
