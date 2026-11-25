#!/usr/bin/env bash
#SBATCH --job-name=gb-wrk
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --output=logs/slurm/workers/%x-%A_%a.out
#SBATCH --error=logs/slurm/workers/%x-%A_%a.err

set -euo pipefail
umask 077
JOB_ENV="${GAMMABOARD_JOB_ENV_FILE:-}"
if [[ -n "$JOB_ENV" ]]; then
    printf '%s  %s\n' "${GAMMABOARD_JOB_ENV_SHA256:?environment snapshot digest required}" "$JOB_ENV" | sha256sum --check --status
    source "$JOB_ENV"
fi
MANIFEST="${GAMMABOARD_MANIFEST:?deployment recipe required}"
exec python3 "$(dirname "$MANIFEST")/launcher/slurm.py" run-worker --spec "${GAMMABOARD_WORKER_SPEC:?deployment recipe required}"
