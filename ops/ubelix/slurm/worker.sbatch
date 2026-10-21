#!/usr/bin/env bash
#SBATCH --job-name=gb-wrk
#SBATCH --time=00:30:00
#SBATCH --signal=B:TERM@60
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1
#SBATCH --mem=2G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --qos=job_gratis
#SBATCH --output=logs/slurm/workers/%x-%A_%a.out
#SBATCH --error=logs/slurm/workers/%x-%A_%a.err

set -euo pipefail
umask 077
PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "$PRIVATE_ENV" ]]; then source "$PRIVATE_ENV"; fi
MANIFEST="${GAMMABOARD_MANIFEST:?deployment recipe required}"
exec python3 "$(dirname "$MANIFEST")/launcher/ubelix.py" run-worker --spec "${GAMMABOARD_WORKER_SPEC:?deployment recipe required}"
