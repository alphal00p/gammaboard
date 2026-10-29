#!/usr/bin/env bash
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err
set -euo pipefail
umask 077
PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "$PRIVATE_ENV" ]]; then source "$PRIVATE_ENV"; fi
RECORD="${GAMMABOARD_BUILD_RECORD:?immutable build record required}"
exec python3 "$(dirname "$RECORD")/recipe/ubelix.py" run-build --record "$RECORD"
