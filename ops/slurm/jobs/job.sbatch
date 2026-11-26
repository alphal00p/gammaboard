#!/usr/bin/env bash
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --output=logs/slurm/%x-%j.out
#SBATCH --error=logs/slurm/%x-%j.err

set -euo pipefail
umask 077
JOB_ENV="${GAMMABOARD_JOB_ENV_FILE:-}"
if [[ -n "$JOB_ENV" ]]; then
    printf '%s  %s\n' "${GAMMABOARD_JOB_ENV_SHA256:?environment snapshot digest required}" "$JOB_ENV" | sha256sum --check --status
    source "$JOB_ENV"
fi
case "${GAMMABOARD_JOB_ROLE:?job role required}" in
    control)
        job_input="${GAMMABOARD_MANIFEST:?deployment recipe required}"
        job_helper="$(dirname "$job_input")/launcher/slurm.py"
        job_flag=--manifest
        ;;
    worker)
        job_input="${GAMMABOARD_WORKER_SPEC:?worker recipe required}"
        job_helper="$(dirname "${GAMMABOARD_MANIFEST:?deployment recipe required}")/launcher/slurm.py"
        job_flag=--spec
        ;;
    build)
        job_input="${GAMMABOARD_BUILD_RECORD:?build record required}"
        job_helper="$(dirname "$job_input")/recipe/slurm.py"
        job_flag=--record
        ;;
    *) echo "Unknown GammaBoard job role: $GAMMABOARD_JOB_ROLE" >&2; exit 2 ;;
esac
exec python3 "$job_helper" "run-$GAMMABOARD_JOB_ROLE" "$job_flag" "$job_input"
