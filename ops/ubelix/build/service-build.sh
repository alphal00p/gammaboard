#!/usr/bin/env bash
set -euo pipefail
umask 077
family="${1:?service family required}"
case "$family" in gammaboard|gammaloop) ;; *) exit 2 ;; esac
workspace="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"
source "${workspace}/ops/build/common.sh"
require_cmd apptainer
require_cmd git
require_cmd python3
require_cmd sha256sum
# Explicit container environment overrides bypass --cleanenv. Do not inherit
# runtime settings (including deployment identity) into release validation.
for variable in ${!APPTAINERENV_@} ${!SINGULARITYENV_@}; do unset "$variable"; done
if [[ "$family" == gammaboard ]]; then
    repo="${GAMMABOARD_REPO:-https://github.com/alphal00p/gammaboard.git}"
    revision="${GAMMABOARD_REVISION:-HEAD}"
else
    repo="${GAMMALOOP_REPO:-https://github.com/alphal00p/gammaloop.git}"
    revision="${GAMMALOOP_REVISION:-HEAD}"
fi
sha="$(git_resolve_sha "$repo" "$revision")"
mkdir -p "${workspace}/artifacts/builds" "${workspace}/artifacts/download-cache" "${workspace}/images/${family}"
build_dir="$(mktemp -d "${workspace}/artifacts/builds/${family}-${SLURM_JOB_ID:-local}-XXXXXX")"
source_dir="${build_dir}/src"
git init "$source_dir"
git -C "$source_dir" remote add origin "$repo"
git -C "$source_dir" fetch --depth 1 origin "$sha"
git -C "$source_dir" checkout --detach FETCH_HEAD
[[ "$(git -C "$source_dir" rev-parse HEAD)" == "$sha" ]]
[[ -z "$(git -C "$source_dir" status --porcelain)" ]]
python3 "${workspace}/ops/build/render.py" "$workspace" "$build_dir" "$family" "$sha" "${SLURM_CPUS_PER_TASK:-4}"
# Each build has private source, target, staging, and temporary image paths.
# Only dependency downloads are shared. Host-native binaries/libraries are never copied.
export APPTAINER_TMPDIR="${build_dir}/tmp"
export APPTAINER_CACHEDIR="${workspace}/artifacts/download-cache/apptainer"
mkdir -p "$APPTAINER_TMPDIR" "$APPTAINER_CACHEDIR"
apptainer build --bind "${workspace}/artifacts/download-cache:/cache" "${build_dir}/image.sif" "${build_dir}/service.def"
if [[ "$family" == gammaboard ]]; then
    apptainer exec --cleanenv "${build_dir}/image.sif" gammaboard build-info > "${build_dir}/build-info.json"
    apptainer exec --cleanenv --bind "${build_dir}:/smoke" "${build_dir}/image.sif" bash /smoke/smoke.sh
else
    apptainer exec --cleanenv "${build_dir}/image.sif" gammaloop --help > "${build_dir}/smoke.log"
    printf '{"build_id":"%s"}\n' "$sha" > "${build_dir}/build-info.json"
    printf 'passed\n' > "${build_dir}/smoke.result"
fi
python3 "${workspace}/ops/build/publish.py" "$workspace" "$build_dir" "$family" "$sha" "$revision"
