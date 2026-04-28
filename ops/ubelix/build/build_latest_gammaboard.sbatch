#!/usr/bin/env bash
#SBATCH --job-name=gb-build-latest
#SBATCH --time=00:45:00
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=6
#SBATCH --mem=8G
#SBATCH --account=gratis
#SBATCH --partition=epyc2
#SBATCH --output=/storage/research/itp_localunitaritydata/logs/build/%x-%j.out
#SBATCH --error=/storage/research/itp_localunitaritydata/logs/build/%x-%j.err

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
LIB_DIR_REL="artifacts/bin/lib"
MIGRATIONS_DIR_REL="artifacts/migrations"
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
LIB_DIR="${WORKSPACE_ROOT}/${LIB_DIR_REL}"
MIGRATIONS_DIR="${WORKSPACE_ROOT}/${MIGRATIONS_DIR_REL}"
SQLX_ROOT_DIR="${WORKSPACE_ROOT}/artifacts/sqlx-root"
RUST_TARGET_BASE="${RUST_TARGET_BASE:-/scratch/network/users/${USER}/gammaboard-target}"
RUST_TARGET_DIR="${RUST_TARGET_BASE}/target"
GL_LATEST_LINK="${GL_IMAGE_DIR}/gammaloop-latest.sif"
GB_LATEST_LINK="${GB_IMAGE_DIR}/gammaboard-latest.sif"

mkdir -p \
  "${WORKSPACE_ROOT}/logs/build" \
  "${TMP_BASE}" \
  "${CACHE_BASE}" \
  "${GL_IMAGE_DIR}" \
  "${GB_IMAGE_DIR}"
mkdir -p "${BIN_DIR}" "${LIB_DIR}" "${MIGRATIONS_DIR}" "${RUST_TARGET_DIR}" "${SQLX_ROOT_DIR}"

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
HOST_SQLX_BIN="${BIN_DIR}/sqlx"
HOST_MIGRATIONS_DIR="${MIGRATIONS_DIR}"
GL_RUST_PROFILE="release"
GL_RUST_FEATURES="ufo_support"
GB_RUST_PROFILE="release"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "gammaloop_def: ${GL_DEF_PATH}"
echo "gammaboard_def: ${GB_DEF_PATH}"
echo "gammaloop_source: ${GL_SRC_DIR}"
echo "gammaboard_source: ${GB_SRC_DIR}"
echo "target_dir_compile: ${RUST_TARGET_DIR}"
echo "gammaloop_bin: ${HOST_GAMMALOOP_BIN}"
echo "gammaboard_bin: ${HOST_GAMMABOARD_BIN}"
echo "sqlx_bin: ${HOST_SQLX_BIN}"
echo "migrations_dir: ${HOST_MIGRATIONS_DIR}"
echo "lib_dir: ${LIB_DIR}"
echo "gammaloop_sha: ${GAMMALOOP_SHA}"
echo "gammaboard_sha: ${GAMMABOARD_SHA}"
echo "gammaloop_image: ${GL_OUT_IMAGE}"
echo "gammaboard_image: ${GB_OUT_IMAGE}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"

if [[ ! -d "${GL_SRC_DIR}/.git" ]]; then
  git clone "${GAMMALOOP_REPO}" "${GL_SRC_DIR}"
fi
TOTAL_START="$(date +%s)"
FETCH_START="$(date +%s)"
git -C "${GL_SRC_DIR}" fetch --depth 1 origin "${GAMMALOOP_SHA}"
git -C "${GL_SRC_DIR}" checkout --detach "${GAMMALOOP_SHA}"
if git -C "${GL_SRC_DIR}" diff --quiet && git -C "${GL_SRC_DIR}" diff --cached --quiet; then
  GAMMALOOP_GIT_DIRTY="false"
else
  GAMMALOOP_GIT_DIRTY="true"
fi

if [[ ! -d "${GB_SRC_DIR}/.git" ]]; then
  git clone "${GAMMABOARD_REPO}" "${GB_SRC_DIR}"
fi
git -C "${GB_SRC_DIR}" fetch --depth 1 origin "${GAMMABOARD_SHA}"
git -C "${GB_SRC_DIR}" checkout --detach "${GAMMABOARD_SHA}"
FETCH_END="$(date +%s)"
if git -C "${GB_SRC_DIR}" diff --quiet && git -C "${GB_SRC_DIR}" diff --cached --quiet; then
  GAMMABOARD_GIT_DIRTY="false"
else
  GAMMABOARD_GIT_DIRTY="true"
fi
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"

echo "building native gammaloop binary"
CARGO_GL_START="$(date +%s)"
(
  cd "${GL_SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" \
    cargo build -p gammaloop-api --bin gammaloop --features "${GL_RUST_FEATURES}" --profile "${GL_RUST_PROFILE}"
)
CARGO_GL_END="$(date +%s)"

echo "building native gammaboard binary"
CARGO_GB_START="$(date +%s)"
(
  cd "${GB_SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" cargo build --release
)
CARGO_GB_END="$(date +%s)"
BIN_STAGE_START="$(date +%s)"
install -Dm755 "${RUST_TARGET_DIR}/${GL_RUST_PROFILE}/gammaloop" "${HOST_GAMMALOOP_BIN}"
install -Dm755 "${RUST_TARGET_DIR}/release/gammaboard" "${HOST_GAMMABOARD_BIN}"
if [[ ! -x "${SQLX_ROOT_DIR}/bin/sqlx" ]]; then
  echo "building sqlx-cli binary"
  cargo install sqlx-cli --locked --no-default-features --features rustls,postgres --root "${SQLX_ROOT_DIR}"
fi
install -Dm755 "${SQLX_ROOT_DIR}/bin/sqlx" "${HOST_SQLX_BIN}"
PY_LIB_PATH="$(ldd "${HOST_GAMMALOOP_BIN}" | awk '/libpython3\.[0-9]+\.so/{print $3; exit}')"
if [[ -z "${PY_LIB_PATH}" || ! -f "${PY_LIB_PATH}" ]]; then
  echo "failed to resolve host libpython from ldd for ${HOST_GAMMALOOP_BIN}" >&2
  exit 1
fi
cp -L "${PY_LIB_PATH}" "${LIB_DIR}/"
rm -rf "${MIGRATIONS_DIR}"
cp -a "${GB_SRC_DIR}/migrations" "${MIGRATIONS_DIR}"
BIN_STAGE_END="$(date +%s)"

# Always build both targets: gammaloop first, then gammaboard.
echo "building image: ${GL_OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
APPTAINER_GL_START="$(date +%s)"
(
  cd "${GL_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${GL_TMP_IMAGE}" "${GL_DEF_FILE_NAME}"
)
mv -f "${GL_TMP_IMAGE}" "${GL_OUT_IMAGE}"
APPTAINER_GL_END="$(date +%s)"

echo "building image: ${GB_OUT_IMAGE}"
export APPTAINERENV_GAMMALOOP_BIN="/workspace/${BIN_DIR_REL}/gammaloop"
export APPTAINERENV_GAMMABOARD_BIN="/workspace/${BIN_DIR_REL}/gammaboard"
export APPTAINERENV_SQLX_BIN="/workspace/${BIN_DIR_REL}/sqlx"
export APPTAINERENV_MIGRATIONS_DIR="/workspace/${MIGRATIONS_DIR_REL}"
APPTAINER_GB_START="$(date +%s)"
(
  cd "${GB_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${GB_TMP_IMAGE}" "${GB_DEF_FILE_NAME}"
)
mv -f "${GB_TMP_IMAGE}" "${GB_OUT_IMAGE}"
APPTAINER_GB_END="$(date +%s)"
TOTAL_END="$(date +%s)"

GL_IMAGE_SIZE_BYTES="$(stat -c '%s' "${GL_OUT_IMAGE}")"
GB_IMAGE_SIZE_BYTES="$(stat -c '%s' "${GB_OUT_IMAGE}")"
if command -v sha256sum >/dev/null 2>&1; then
  GL_IMAGE_SHA256="$(sha256sum "${GL_OUT_IMAGE}" | awk '{print $1}')"
  GB_IMAGE_SHA256="$(sha256sum "${GB_OUT_IMAGE}" | awk '{print $1}')"
else
  GL_IMAGE_SHA256="-"
  GB_IMAGE_SHA256="-"
fi

TOTAL_SECONDS="$(seconds_since "${TOTAL_START}" "${TOTAL_END}")"
FETCH_SECONDS="$(seconds_since "${FETCH_START}" "${FETCH_END}")"
CARGO_GL_SECONDS="$(seconds_since "${CARGO_GL_START}" "${CARGO_GL_END}")"
CARGO_GB_SECONDS="$(seconds_since "${CARGO_GB_START}" "${CARGO_GB_END}")"
BIN_STAGE_SECONDS="$(seconds_since "${BIN_STAGE_START}" "${BIN_STAGE_END}")"
APPTAINER_GL_SECONDS="$(seconds_since "${APPTAINER_GL_START}" "${APPTAINER_GL_END}")"
APPTAINER_GB_SECONDS="$(seconds_since "${APPTAINER_GB_START}" "${APPTAINER_GB_END}")"

write_meta "${GL_META_FILE}" \
kind=gammaloop \
def_path="${GAMMALOOP_DEF_REL}" \
gammaloop_repo="${GAMMALOOP_REPO}" \
gammaloop_sha="${GAMMALOOP_SHA}" \
gammaloop_git_dirty="${GAMMALOOP_GIT_DIRTY}" \
slurm_job_id="${SLURM_JOB_ID:-}" \
build_host="${HOSTNAME}" \
rustc_version="${RUSTC_VERSION}" \
cargo_version="${CARGO_VERSION}" \
cargo_profile="${GL_RUST_PROFILE}" \
cargo_features="${GL_RUST_FEATURES}" \
rust_target_dir="${RUST_TARGET_DIR}" \
image_size_bytes="${GL_IMAGE_SIZE_BYTES}" \
image_sha256="${GL_IMAGE_SHA256}" \
timing_total_seconds="${TOTAL_SECONDS}" \
timing_fetch_checkout_seconds="${FETCH_SECONDS}" \
timing_cargo_gammaloop_seconds="${CARGO_GL_SECONDS}" \
timing_cargo_gammaboard_seconds="${CARGO_GB_SECONDS}" \
timing_binary_stage_seconds="${BIN_STAGE_SECONDS}" \
timing_apptainer_gammaloop_seconds="${APPTAINER_GL_SECONDS}" \
timing_apptainer_gammaboard_seconds="${APPTAINER_GB_SECONDS}"

write_meta "${GB_META_FILE}" \
kind=gammaboard \
def_path="${GAMMABOARD_DEF_REL}" \
gammaboard_repo="${GAMMABOARD_REPO}" \
gammaboard_sha="${GAMMABOARD_SHA}" \
gammaboard_git_dirty="${GAMMABOARD_GIT_DIRTY}" \
gammaloop_repo="${GAMMALOOP_REPO}" \
gammaloop_sha="${GAMMALOOP_SHA}" \
gammaloop_git_dirty="${GAMMALOOP_GIT_DIRTY}" \
slurm_job_id="${SLURM_JOB_ID:-}" \
build_host="${HOSTNAME}" \
rustc_version="${RUSTC_VERSION}" \
cargo_version="${CARGO_VERSION}" \
cargo_profile="${GB_RUST_PROFILE}" \
cargo_features="-" \
rust_target_dir="${RUST_TARGET_DIR}" \
image_size_bytes="${GB_IMAGE_SIZE_BYTES}" \
image_sha256="${GB_IMAGE_SHA256}" \
timing_total_seconds="${TOTAL_SECONDS}" \
timing_fetch_checkout_seconds="${FETCH_SECONDS}" \
timing_cargo_gammaloop_seconds="${CARGO_GL_SECONDS}" \
timing_cargo_gammaboard_seconds="${CARGO_GB_SECONDS}" \
timing_binary_stage_seconds="${BIN_STAGE_SECONDS}" \
timing_apptainer_gammaloop_seconds="${APPTAINER_GL_SECONDS}" \
timing_apptainer_gammaboard_seconds="${APPTAINER_GB_SECONDS}"

ln -sfn "$(basename "${GL_OUT_IMAGE}")" "${GL_LATEST_LINK}"
ln -sfn "$(basename "${GB_OUT_IMAGE}")" "${GB_LATEST_LINK}"

echo "updated symlink: ${GL_LATEST_LINK} -> $(readlink "${GL_LATEST_LINK}")"
echo "updated symlink: ${GB_LATEST_LINK} -> $(readlink "${GB_LATEST_LINK}")"
