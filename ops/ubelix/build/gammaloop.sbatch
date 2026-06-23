#!/usr/bin/env bash
#SBATCH --job-name=gl-build
#SBATCH --time=00:30:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=6
#SBATCH --mem=8G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=logs/slurm/build/%x-%j.out
#SBATCH --error=logs/slurm/build/%x-%j.err

set -euo pipefail

PRIVATE_ENV="${HOME}/.config/gammaboard/slurm.env"
if [[ -f "${PRIVATE_ENV}" ]]; then
  # Load per-user secrets and cluster-local overrides without tracking them in git.
  source "${PRIVATE_ENV}"
fi
unset NO_SYMBOLICA_OEM_LICENSE
unset APPTAINERENV_NO_SYMBOLICA_OEM_LICENSE
unset APPTAINERENV_SYMBOLICA_LICENSE

WORKSPACE_ROOT="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"
GAMMALOOP_REPO="${GAMMALOOP_REPO:-https://github.com/alphal00p/gammaloop.git}"
DEF_REL_PATH="ops/build/gammaloop.def"
IMAGE_DIR_REL="images/gammaloop"
SRC_DIR_REL="artifacts/src/gammaloop"
BIN_DIR_REL="artifacts/bin"
LIB_DIR_REL="artifacts/bin/lib"
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

seconds_since() {
  local start="$1"
  local end="$2"
  printf '%s\n' "$((end - start))"
}

require_cmd apptainer
require_cmd git
require_cmd awk
require_cmd cargo
require_cmd stat

TMP_BASE="${SCRATCH_BASE}/tmp"
CACHE_BASE="${SCRATCH_BASE}/cache"
DEF_PATH="${WORKSPACE_ROOT}/${DEF_REL_PATH}"
DEF_DIR="$(dirname "${DEF_PATH}")"
DEF_FILE_NAME="$(basename "${DEF_PATH}")"
IMAGE_DIR="${WORKSPACE_ROOT}/${IMAGE_DIR_REL}"
SRC_DIR="${WORKSPACE_ROOT}/${SRC_DIR_REL}"
BIN_DIR="${WORKSPACE_ROOT}/${BIN_DIR_REL}"
LIB_DIR="${WORKSPACE_ROOT}/${LIB_DIR_REL}"
RUST_TARGET_BASE="${RUST_TARGET_BASE:-/scratch/network/users/${USER}/gammaboard-target}"
RUST_TARGET_DIR="${RUST_TARGET_BASE}/target"
CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-1}"

mkdir -p "${WORKSPACE_ROOT}/logs/slurm/build" "${TMP_BASE}" "${CACHE_BASE}" "${IMAGE_DIR}" "${BIN_DIR}" "${LIB_DIR}" "${RUST_TARGET_DIR}"

if [[ ! -f "${DEF_PATH}" ]]; then
  echo "missing def file: ${DEF_PATH}" >&2
  exit 1
fi

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"
export CARGO_INCREMENTAL
export SYMBOLICA_OEM_LICENSE="${SYMBOLICA_OEM_LICENSE:-SYMBOLICA_OEM_GAMMALOOP}"
if [[ ! "${SYMBOLICA_OEM_LICENSE}" == SYMBOLICA_OEM_* ]]; then
  echo "SYMBOLICA_OEM_LICENSE must start with SYMBOLICA_OEM_; got '${SYMBOLICA_OEM_LICENSE}'" >&2
  exit 1
fi

GAMMALOOP_SHA="$(git_head_sha "${GAMMALOOP_REPO}")"
OUT_IMAGE="${IMAGE_DIR}/gammaloop.sif"
TMP_IMAGE="${IMAGE_DIR}/.gammaloop.tmp-${SLURM_JOB_ID:-$$}.sif"
META_FILE="${IMAGE_DIR}/gammaloop.meta"
HOST_GAMMALOOP_BIN="${BIN_DIR}/gammaloop"
RUST_PROFILE="release"
RUST_FEATURES="ufo_support"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "def_path: ${DEF_PATH}"
echo "source_dir: ${SRC_DIR}"
echo "target_dir_compile: ${RUST_TARGET_DIR}"
echo "bin_path: ${HOST_GAMMALOOP_BIN}"
echo "lib_dir: ${LIB_DIR}"
echo "gammaloop_sha: ${GAMMALOOP_SHA}"
echo "out_image: ${OUT_IMAGE}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"
echo "CARGO_INCREMENTAL: ${CARGO_INCREMENTAL}"
echo "SYMBOLICA_OEM_LICENSE: ${SYMBOLICA_OEM_LICENSE}"

if [[ ! -d "${SRC_DIR}/.git" ]]; then
  git clone "${GAMMALOOP_REPO}" "${SRC_DIR}"
fi
TOTAL_START="$(date +%s)"
FETCH_START="$(date +%s)"
git -C "${SRC_DIR}" fetch --depth 1 origin "${GAMMALOOP_SHA}"
git -C "${SRC_DIR}" checkout --detach "${GAMMALOOP_SHA}"
FETCH_END="$(date +%s)"
if git -C "${SRC_DIR}" diff --quiet && git -C "${SRC_DIR}" diff --cached --quiet; then
  GAMMALOOP_GIT_DIRTY="false"
else
  GAMMALOOP_GIT_DIRTY="true"
fi
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"

echo "building native gammaloop binary"
CARGO_START="$(date +%s)"
(
  cd "${SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" \
    cargo build -p gammaloop-api --bin gammaloop --features "${RUST_FEATURES}" --profile "${RUST_PROFILE}"
)
CARGO_END="$(date +%s)"
BIN_STAGE_START="$(date +%s)"
install -Dm755 "${RUST_TARGET_DIR}/${RUST_PROFILE}/gammaloop" "${HOST_GAMMALOOP_BIN}"
PY_LIB_PATH="$(ldd "${HOST_GAMMALOOP_BIN}" | awk '/libpython3\.[0-9]+\.so/{print $3; exit}')"
if [[ -z "${PY_LIB_PATH}" || ! -f "${PY_LIB_PATH}" ]]; then
  echo "failed to resolve host libpython from ldd for ${HOST_GAMMALOOP_BIN}" >&2
  exit 1
fi
cp -L "${PY_LIB_PATH}" "${LIB_DIR}/"
BIN_STAGE_END="$(date +%s)"

echo "building image: ${OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
APPTAINER_START="$(date +%s)"
(
  cd "${DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${TMP_IMAGE}" "${DEF_FILE_NAME}"
)
mv -f "${TMP_IMAGE}" "${OUT_IMAGE}"
APPTAINER_END="$(date +%s)"
TOTAL_END="$(date +%s)"

IMAGE_SIZE_BYTES="$(stat -c '%s' "${OUT_IMAGE}")"
if command -v sha256sum >/dev/null 2>&1; then
  IMAGE_SHA256="$(sha256sum "${OUT_IMAGE}" | awk '{print $1}')"
else
  IMAGE_SHA256="-"
fi

TOTAL_SECONDS="$(seconds_since "${TOTAL_START}" "${TOTAL_END}")"
FETCH_SECONDS="$(seconds_since "${FETCH_START}" "${FETCH_END}")"
CARGO_SECONDS="$(seconds_since "${CARGO_START}" "${CARGO_END}")"
BIN_STAGE_SECONDS="$(seconds_since "${BIN_STAGE_START}" "${BIN_STAGE_END}")"
APPTAINER_TOTAL_SECONDS="$(seconds_since "${APPTAINER_START}" "${APPTAINER_END}")"

write_meta "${META_FILE}" \
kind=gammaloop \
def_path="${DEF_REL_PATH}" \
gammaloop_repo="${GAMMALOOP_REPO}" \
gammaloop_sha="${GAMMALOOP_SHA}" \
gammaloop_git_dirty="${GAMMALOOP_GIT_DIRTY}" \
slurm_job_id="${SLURM_JOB_ID:-}" \
build_host="${HOSTNAME}" \
rustc_version="${RUSTC_VERSION}" \
cargo_version="${CARGO_VERSION}" \
cargo_profile="${RUST_PROFILE}" \
cargo_features="${RUST_FEATURES}" \
rust_target_dir="${RUST_TARGET_DIR}" \
cargo_incremental="${CARGO_INCREMENTAL}" \
image_size_bytes="${IMAGE_SIZE_BYTES}" \
image_sha256="${IMAGE_SHA256}" \
timing_total_seconds="${TOTAL_SECONDS}" \
timing_fetch_checkout_seconds="${FETCH_SECONDS}" \
timing_cargo_gammaloop_seconds="${CARGO_SECONDS}" \
timing_binary_stage_seconds="${BIN_STAGE_SECONDS}" \
timing_apptainer_gammaloop_seconds="${APPTAINER_TOTAL_SECONDS}"
