#!/usr/bin/env bash
#SBATCH --job-name=gb-build-latest
#SBATCH --time=02:00:00
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
GAMMABOARD_REPO="${GAMMABOARD_REPO:-https://github.com/SecretGmG/gammaboard.git}"
GAMMALOOP_DEF_REL="ops/build/gammaloop.def"
GAMMABOARD_DEF_REL="ops/build/gammaboard.def"
GL_IMAGE_DIR_REL="images/gammaloop"
GB_IMAGE_DIR_REL="images/gammaboard"
GL_SRC_DIR_REL="artifacts/src/gammaloop"
GB_SRC_DIR_REL="artifacts/src/gammaboard"
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

write_meta() {
  local meta_path="$1"
  shift
  : >"${meta_path}"
  for line in "$@"; do
    printf '%s\n' "${line}" >>"${meta_path}"
  done
  printf 'built_at_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${meta_path}"
}

require_cmd apptainer
require_cmd git
require_cmd awk
require_cmd cargo

TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"

GL_DEF_PATH="${WORKSPACE_ROOT}/${GAMMALOOP_DEF_REL}"
GB_DEF_PATH="${WORKSPACE_ROOT}/${GAMMABOARD_DEF_REL}"
GL_DEF_DIR="$(dirname "${GL_DEF_PATH}")"
GB_DEF_DIR="$(dirname "${GB_DEF_PATH}")"
GL_DEF_FILE_NAME="$(basename "${GL_DEF_PATH}")"
GB_DEF_FILE_NAME="$(basename "${GB_DEF_PATH}")"
GL_IMAGE_DIR="${WORKSPACE_ROOT}/${GL_IMAGE_DIR_REL}"
GB_IMAGE_DIR="${WORKSPACE_ROOT}/${GB_IMAGE_DIR_REL}"
GL_SRC_DIR="${WORKSPACE_ROOT}/${GL_SRC_DIR_REL}"
GB_SRC_DIR="${WORKSPACE_ROOT}/${GB_SRC_DIR_REL}"
BIN_DIR="${WORKSPACE_ROOT}/${BIN_DIR_REL}"
TARGET_DIR="${WORKSPACE_ROOT}/${TARGET_DIR_REL}"
RUST_TARGET_BASE="${RUST_TARGET_BASE:-/scratch/network/users/${USER}/gammaboard-target}"
RUST_TARGET_DIR="${RUST_TARGET_BASE}/target"
GL_LATEST_LINK="${GL_IMAGE_DIR}/gammaloop-latest.sif"
GB_LATEST_LINK="${GB_IMAGE_DIR}/gammaboard-latest.sif"

mkdir -p \
  "logs/build" \
  "${WORKSPACE_ROOT}/logs/build" \
  "${TMP_BASE}" \
  "${CACHE_BASE}" \
  "${GL_IMAGE_DIR}" \
  "${GB_IMAGE_DIR}"
mkdir -p "${BIN_DIR}" "${TARGET_DIR}" "${RUST_TARGET_DIR}"

if [[ ! -f "${GL_DEF_PATH}" ]]; then
  echo "missing def file: ${GL_DEF_PATH}" >&2
  exit 1
fi
if [[ ! -f "${GB_DEF_PATH}" ]]; then
  echo "missing def file: ${GB_DEF_PATH}" >&2
  exit 1
fi

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"

GAMMALOOP_SHA="$(git_head_sha "${GAMMALOOP_REPO}")"
GAMMABOARD_SHA="$(git_head_sha "${GAMMABOARD_REPO}")"

GL_OUT_IMAGE="${GL_IMAGE_DIR}/gammaloop-${GAMMALOOP_SHA}.sif"
GB_OUT_IMAGE="${GB_IMAGE_DIR}/gammaboard-${GAMMABOARD_SHA}.sif"
GL_TMP_IMAGE="${GL_IMAGE_DIR}/.gammaloop-${GAMMALOOP_SHA}.tmp-${SLURM_JOB_ID:-$$}.sif"
GB_TMP_IMAGE="${GB_IMAGE_DIR}/.gammaboard-${GAMMABOARD_SHA}.tmp-${SLURM_JOB_ID:-$$}.sif"
GL_META_FILE="${GL_IMAGE_DIR}/gammaloop-${GAMMALOOP_SHA}.meta"
GB_META_FILE="${GB_IMAGE_DIR}/gammaboard-${GAMMABOARD_SHA}.meta"
HOST_GAMMALOOP_BIN="${BIN_DIR}/gammaloop"
HOST_GAMMABOARD_BIN="${BIN_DIR}/gammaboard"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "gammaloop_def: ${GL_DEF_PATH}"
echo "gammaboard_def: ${GB_DEF_PATH}"
echo "gammaloop_source: ${GL_SRC_DIR}"
echo "gammaboard_source: ${GB_SRC_DIR}"
echo "target_dir_workspace_note: ${TARGET_DIR}"
echo "target_dir_compile: ${RUST_TARGET_DIR}"
echo "gammaloop_bin: ${HOST_GAMMALOOP_BIN}"
echo "gammaboard_bin: ${HOST_GAMMABOARD_BIN}"
echo "gammaloop_sha: ${GAMMALOOP_SHA}"
echo "gammaboard_sha: ${GAMMABOARD_SHA}"
echo "gammaloop_image: ${GL_OUT_IMAGE}"
echo "gammaboard_image: ${GB_OUT_IMAGE}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

if [[ ! -d "${GL_SRC_DIR}/.git" ]]; then
  git clone "${GAMMALOOP_REPO}" "${GL_SRC_DIR}"
fi
git -C "${GL_SRC_DIR}" fetch --depth 1 origin "${GAMMALOOP_SHA}"
git -C "${GL_SRC_DIR}" checkout --detach "${GAMMALOOP_SHA}"

if [[ ! -d "${GB_SRC_DIR}/.git" ]]; then
  git clone "${GAMMABOARD_REPO}" "${GB_SRC_DIR}"
fi
git -C "${GB_SRC_DIR}" fetch --depth 1 origin "${GAMMABOARD_SHA}"
git -C "${GB_SRC_DIR}" checkout --detach "${GAMMABOARD_SHA}"

echo "building native gammaloop binary"
(
  cd "${GL_SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" \
    cargo build -p gammaloop-api --bin gammaloop --features ufo_support --profile dev-optim
)
install -Dm755 "${RUST_TARGET_DIR}/dev-optim/gammaloop" "${HOST_GAMMALOOP_BIN}"

echo "building native gammaboard binary"
(
  cd "${GB_SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" cargo build --release
)
install -Dm755 "${RUST_TARGET_DIR}/release/gammaboard" "${HOST_GAMMABOARD_BIN}"

# Always build both targets: gammaloop first, then gammaboard.
echo "building image: ${GL_OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
(
  cd "${GL_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${GL_TMP_IMAGE}" "${GL_DEF_FILE_NAME}"
)
mv -f "${GL_TMP_IMAGE}" "${GL_OUT_IMAGE}"

echo "building image: ${GB_OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
export APPTAINERENV_GAMMABOARD_BIN="/workspace/${BIN_DIR_REL}/gammaboard"
(
  cd "${GB_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${GB_TMP_IMAGE}" "${GB_DEF_FILE_NAME}"
)
mv -f "${GB_TMP_IMAGE}" "${GB_OUT_IMAGE}"

write_meta "${GL_META_FILE}" \
kind=gammaloop \
def_path="${GAMMALOOP_DEF_REL}" \
gammaloop_repo="${GAMMALOOP_REPO}" \
gammaloop_sha="${GAMMALOOP_SHA}"

write_meta "${GB_META_FILE}" \
kind=gammaboard \
def_path="${GAMMABOARD_DEF_REL}" \
gammaboard_repo="${GAMMABOARD_REPO}" \
gammaboard_sha="${GAMMABOARD_SHA}" \
gammaloop_repo="${GAMMALOOP_REPO}" \
gammaloop_sha="${GAMMALOOP_SHA}"

ln -sfn "$(basename "${GL_OUT_IMAGE}")" "${GL_LATEST_LINK}"
ln -sfn "$(basename "${GB_OUT_IMAGE}")" "${GB_LATEST_LINK}"

echo "updated symlink: ${GL_LATEST_LINK} -> $(readlink "${GL_LATEST_LINK}")"
echo "updated symlink: ${GB_LATEST_LINK} -> $(readlink "${GB_LATEST_LINK}")"
