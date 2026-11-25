#!/usr/bin/env bash
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err
set -euo pipefail
umask 077
JOB_ENV="${GAMMABOARD_JOB_ENV_FILE:-}"
if [[ -n "$JOB_ENV" ]]; then
    printf '%s  %s\n' "${GAMMABOARD_JOB_ENV_SHA256:?environment snapshot digest required}" "$JOB_ENV" | sha256sum --check --status
    source "$JOB_ENV"
fi
RECORD="${GAMMABOARD_BUILD_RECORD:?immutable build record required}"
exec python3 "$(dirname "$RECORD")/recipe/slurm.py" run-build --record "$RECORD"
