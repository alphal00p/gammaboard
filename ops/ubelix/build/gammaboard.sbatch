#!/usr/bin/env bash
#SBATCH --job-name=gb-build
#SBATCH --time=00:45:00
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
GAMMABOARD_REPO="${GAMMABOARD_REPO:-https://github.com/alphal00p/gammaboard.git}"
GAMMABOARD_DEF_REL="ops/build/gammaboard.def"
GB_IMAGE_DIR_REL="images/gammaboard"
GB_SRC_DIR_REL="artifacts/src/gammaboard"
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

GB_DEF_PATH="${WORKSPACE_ROOT}/${GAMMABOARD_DEF_REL}"
GB_DEF_DIR="$(dirname "${GB_DEF_PATH}")"
GB_DEF_FILE_NAME="$(basename "${GB_DEF_PATH}")"
GB_IMAGE_DIR="${WORKSPACE_ROOT}/${GB_IMAGE_DIR_REL}"
GB_SRC_DIR="${WORKSPACE_ROOT}/${GB_SRC_DIR_REL}"
BIN_DIR="${WORKSPACE_ROOT}/${BIN_DIR_REL}"
LIB_DIR="${WORKSPACE_ROOT}/${LIB_DIR_REL}"
SQLX_ROOT_DIR="${WORKSPACE_ROOT}/artifacts/sqlx-root"
NPM_CACHE_DIR="${WORKSPACE_ROOT}/artifacts/npm-cache"
RUST_TARGET_BASE="${RUST_TARGET_BASE:-/scratch/network/users/${USER}/gammaboard-target}"
RUST_TARGET_DIR="${RUST_TARGET_BASE}/target"
CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-1}"

mkdir -p \
  "${WORKSPACE_ROOT}/logs/slurm/build" \
  "${TMP_BASE}" \
  "${CACHE_BASE}" \
  "${GB_IMAGE_DIR}"
mkdir -p "${BIN_DIR}" "${LIB_DIR}" "${RUST_TARGET_DIR}" "${SQLX_ROOT_DIR}" "${NPM_CACHE_DIR}"

if [[ ! -f "${GB_DEF_PATH}" ]]; then
  echo "missing def file: ${GB_DEF_PATH}" >&2
  exit 1
fi

export APPTAINER_TMPDIR="${TMP_BASE}"
export APPTAINER_CACHEDIR="${CACHE_BASE}"
export CARGO_INCREMENTAL

GAMMABOARD_SHA="$(git_head_sha "${GAMMABOARD_REPO}")"

GB_OUT_IMAGE="${GB_IMAGE_DIR}/gammaboard.sif"
GB_TMP_IMAGE="${GB_IMAGE_DIR}/.gammaboard.tmp-${SLURM_JOB_ID:-$$}.sif"
GB_META_FILE="${GB_IMAGE_DIR}/gammaboard.meta"
HOST_GAMMABOARD_BIN="${BIN_DIR}/gammaboard"
HOST_SQLX_BIN="${BIN_DIR}/sqlx"
GB_RUST_PROFILE="release"

echo "host: ${HOSTNAME}"
echo "workspace_root: ${WORKSPACE_ROOT}"
echo "gammaboard_def: ${GB_DEF_PATH}"
echo "gammaboard_source: ${GB_SRC_DIR}"
echo "target_dir_compile: ${RUST_TARGET_DIR}"
echo "gammaboard_bin: ${HOST_GAMMABOARD_BIN}"
echo "sqlx_bin: ${HOST_SQLX_BIN}"
echo "migrations_dir: ${GB_SRC_DIR}/migrations"
echo "dashboard_source_dir: ${GB_SRC_DIR}/dashboard"
echo "lib_dir: ${LIB_DIR}"
echo "gammaboard_sha: ${GAMMABOARD_SHA}"
echo "gammaboard_image: ${GB_OUT_IMAGE}"
echo "APPTAINER_TMPDIR: ${APPTAINER_TMPDIR}"
echo "APPTAINER_CACHEDIR: ${APPTAINER_CACHEDIR}"
echo "CARGO_INCREMENTAL: ${CARGO_INCREMENTAL}"
echo "symbolica_license_mode: bundled-oem"

TOTAL_START="$(date +%s)"
FETCH_START="$(date +%s)"
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

echo "building native gammaboard binary"
CARGO_GB_START="$(date +%s)"
(
  cd "${GB_SRC_DIR}"
  CARGO_TARGET_DIR="${RUST_TARGET_DIR}" cargo build --release
)
CARGO_GB_END="$(date +%s)"
BIN_STAGE_START="$(date +%s)"
install -Dm755 "${RUST_TARGET_DIR}/release/gammaboard" "${HOST_GAMMABOARD_BIN}"
if [[ ! -x "${SQLX_ROOT_DIR}/bin/sqlx" ]]; then
  echo "building sqlx-cli binary"
  cargo install sqlx-cli --locked --no-default-features --features rustls,postgres --root "${SQLX_ROOT_DIR}"
fi
install -Dm755 "${SQLX_ROOT_DIR}/bin/sqlx" "${HOST_SQLX_BIN}"
PY_LIB_PATH="$(ldd "${HOST_GAMMABOARD_BIN}" | awk '/libpython3\.[0-9]+\.so/{print $3; exit}')"
if [[ -n "${PY_LIB_PATH}" ]]; then
  if [[ ! -f "${PY_LIB_PATH}" ]]; then
    echo "failed to resolve host libpython from ldd for ${HOST_GAMMABOARD_BIN}" >&2
    exit 1
  fi
  cp -L "${PY_LIB_PATH}" "${LIB_DIR}/"
fi
BIN_STAGE_END="$(date +%s)"

echo "building image: ${GB_OUT_IMAGE}"
export APPTAINERENV_GAMMABOARD_BIN="/workspace/${BIN_DIR_REL}/gammaboard"
export APPTAINERENV_SQLX_BIN="/workspace/${BIN_DIR_REL}/sqlx"
export APPTAINERENV_MIGRATIONS_DIR="/workspace/${GB_SRC_DIR_REL}/migrations"
export APPTAINERENV_DASHBOARD_SOURCE_DIR="/workspace/${GB_SRC_DIR_REL}/dashboard"
export APPTAINERENV_NPM_CONFIG_CACHE="/workspace/artifacts/npm-cache"
APPTAINER_GB_START="$(date +%s)"
(
  cd "${GB_DEF_DIR}"
  apptainer build --notest --bind "${WORKSPACE_ROOT}:/workspace" "${GB_TMP_IMAGE}" "${GB_DEF_FILE_NAME}"
)
mv -f "${GB_TMP_IMAGE}" "${GB_OUT_IMAGE}"
APPTAINER_GB_END="$(date +%s)"
TOTAL_END="$(date +%s)"

GB_IMAGE_SIZE_BYTES="$(stat -c '%s' "${GB_OUT_IMAGE}")"
if command -v sha256sum >/dev/null 2>&1; then
  GB_IMAGE_SHA256="$(sha256sum "${GB_OUT_IMAGE}" | awk '{print $1}')"
else
  GB_IMAGE_SHA256="-"
fi

TOTAL_SECONDS="$(seconds_since "${TOTAL_START}" "${TOTAL_END}")"
FETCH_SECONDS="$(seconds_since "${FETCH_START}" "${FETCH_END}")"
CARGO_GB_SECONDS="$(seconds_since "${CARGO_GB_START}" "${CARGO_GB_END}")"
BIN_STAGE_SECONDS="$(seconds_since "${BIN_STAGE_START}" "${BIN_STAGE_END}")"
APPTAINER_GB_SECONDS="$(seconds_since "${APPTAINER_GB_START}" "${APPTAINER_GB_END}")"

write_meta "${GB_META_FILE}" \
kind=gammaboard \
def_path="${GAMMABOARD_DEF_REL}" \
gammaboard_repo="${GAMMABOARD_REPO}" \
gammaboard_sha="${GAMMABOARD_SHA}" \
gammaboard_git_dirty="${GAMMABOARD_GIT_DIRTY}" \
slurm_job_id="${SLURM_JOB_ID:-}" \
build_host="${HOSTNAME}" \
rustc_version="${RUSTC_VERSION}" \
cargo_version="${CARGO_VERSION}" \
cargo_profile="${GB_RUST_PROFILE}" \
cargo_features="-" \
rust_target_dir="${RUST_TARGET_DIR}" \
cargo_incremental="${CARGO_INCREMENTAL}" \
image_size_bytes="${GB_IMAGE_SIZE_BYTES}" \
image_sha256="${GB_IMAGE_SHA256}" \
timing_total_seconds="${TOTAL_SECONDS}" \
timing_fetch_checkout_seconds="${FETCH_SECONDS}" \
timing_cargo_gammaboard_seconds="${CARGO_GB_SECONDS}" \
timing_binary_stage_seconds="${BIN_STAGE_SECONDS}" \
timing_apptainer_gammaboard_seconds="${APPTAINER_GB_SECONDS}"
