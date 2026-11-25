#!/usr/bin/env bash
#SBATCH --job-name=gb-ctl
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --output=logs/slurm/control/%x-%j.out
#SBATCH --error=logs/slurm/control/%x-%j.err

set -euo pipefail
umask 077
JOB_ENV="${GAMMABOARD_JOB_ENV_FILE:-}"
if [[ -n "$JOB_ENV" ]]; then
    printf '%s  %s\n' "${GAMMABOARD_JOB_ENV_SHA256:?environment snapshot digest required}" "$JOB_ENV" | sha256sum --check --status
    source "$JOB_ENV"
fi
MANIFEST="${GAMMABOARD_MANIFEST:?deployment recipe required}"
exec python3 "$(dirname "$MANIFEST")/launcher/slurm.py" run-control --manifest "$MANIFEST"
