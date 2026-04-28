#!/usr/bin/env bash
#SBATCH --job-name=gl-build-latest
#SBATCH --time=01:00:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=16
#SBATCH --mem=16G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=logs/build/%x-%j.out
#SBATCH --error=logs/build/%x-%j.err

set -euo pipefail

# Edit directly on UBELIX when needed.
WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-/storage/research/itp_localunitaritydata}"
GAMMALOOP_REPO="${GAMMALOOP_REPO:-https://github.com/alphal00p/gammaloop.git}"
DEF_REL_PATH="ops/build/gammaloop.def"
IMAGE_DIR_REL="images/gammaloop"
SRC_DIR_REL="artifacts/src/gammaloop"
BIN_DIR_REL="artifacts/bin"
TARGET_DIR_REL="artifacts/target"
SCRATCH_BASE="${SCRATCH_BASE:-/scratch/network/users/${USER}/apptainer}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

git_head_sha() {
  local repo_url="$1"
  local sha
  sha="$(git ls-remote "${repo_url}" HEAD | awk 'NR==1 {print $1}')"
  if [[ -z "${sha}" ]]; then
    echo "failed to resolve HEAD for ${repo_url}" >&2
    exit 1
  fi
  printf '%s\n' "${sha}"
}

require_cmd apptainer
require_cmd git
require_cmd awk
require_cmd cargo

TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"
DEF_PATH="${WORKSPACE_ROOT}/${DEF_REL_PATH}"
DEF_DIR="$(dirname "${DEF_PATH}")"
DEF_FILE_NAME="$(basename "${DEF_PATH}")"
IMAGE_DIR="${WORKSPACE_ROOT}/${IMAGE_DIR_REL}"
SRC_DIR="${WORKSPACE_ROOT}/${SRC_DIR_REL}"
BIN_DIR="${WORKSPACE_ROOT}/${BIN_DIR_REL}"
TARGET_DIR="${WORKSPACE_ROOT}/${TARGET_DIR_REL}"
RUST_TARGET_BASE="${RUST_TARGET_BASE:-/scratch/network/users/${USER}/gammaboard-target}"
RUST_TARGET_DIR="${RUST_TARGET_BASE}/target"
LATEST_LINK="${IMAGE_DIR}/gammaloop-latest.sif"

mkdir -p "logs/build" "${WORKSPACE_ROOT}/logs/build" "${TMP_BASE}" "${CACHE_BASE}" "${IMAGE_DIR}" "${BIN_DIR}" "${TARGET_DIR}" "${RUST_TARGET_DIR}"

if [[ ! -f "${DEF_PATH}" ]]; then
  echo "missing def file: ${DEF_PATH}" >&2
  exit 1
fi

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"

GAMMALOOP_SHA="$(git_head_sha "${GAMMALOOP_REPO}")"
OUT_IMAGE="${IMAGE_DIR}/gammaloop-${GAMMALOOP_SHA}.sif"
TMP_IMAGE="${IMAGE_DIR}/.gammaloop-${GAMMALOOP_SHA}.tmp-${SLURM_JOB_ID:-$$}.sif"
META_FILE="${IMAGE_DIR}/gammaloop-${GAMMALOOP_SHA}.meta"
HOST_GAMMALOOP_BIN="${BIN_DIR}/gammaloop"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "def_path: ${DEF_PATH}"
echo "source_dir: ${SRC_DIR}"
echo "target_dir_workspace_note: ${TARGET_DIR}"
echo "target_dir_compile: ${RUST_TARGET_DIR}"
echo "bin_path: ${HOST_GAMMALOOP_BIN}"
echo "gammaloop_sha: ${GAMMALOOP_SHA}"
echo "out_image: ${OUT_IMAGE}"
echo "latest_link: ${LATEST_LINK}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

if [[ ! -d "${SRC_DIR}/.git" ]]; then
  git clone "${GAMMALOOP_REPO}" "${SRC_DIR}"
fi
git -C "${SRC_DIR}" fetch --depth 1 origin "${GAMMALOOP_SHA}"
git -C "${SRC_DIR}" checkout --detach "${GAMMALOOP_SHA}"

echo "building native gammaloop binary"
(
  cd "${SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" \
    cargo build -p gammaloop-api --bin gammaloop --features ufo_support --profile dev-optim
)
install -Dm755 "${RUST_TARGET_DIR}/dev-optim/gammaloop" "${HOST_GAMMALOOP_BIN}"

echo "building image: ${OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
(
  cd "${DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${TMP_IMAGE}" "${DEF_FILE_NAME}"
)
mv -f "${TMP_IMAGE}" "${OUT_IMAGE}"

cat >"${META_FILE}" <<EOF
kind=gammaloop
def_path=${DEF_REL_PATH}
gammaloop_repo=${GAMMALOOP_REPO}
gammaloop_sha=${GAMMALOOP_SHA}
built_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

ln -sfn "$(basename "${OUT_IMAGE}")" "${LATEST_LINK}"
echo "updated symlink: ${LATEST_LINK} -> $(readlink "${LATEST_LINK}")"
