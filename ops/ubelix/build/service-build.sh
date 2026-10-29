#!/usr/bin/env bash
set -euo pipefail
umask 077
family="${1:?service family required}"
case "$family" in gammaboard|gammaloop) ;; *) exit 2 ;; esac
workspace="${GAMMABOARD_WORKSPACE_ROOT:-$(pwd -P)}"
recipe="${GAMMABOARD_RECIPE_ROOT:?frozen recipe required}"
build_dir="${GAMMABOARD_BUILD_DIR:?private build directory required}"
for tool in apptainer git python3 sha256sum; do
    command -v "$tool" >/dev/null || { echo "missing required command: $tool" >&2; exit 1; }
done
# Explicit container environment overrides bypass --cleanenv. Do not inherit
# runtime settings (including deployment identity) into release validation.
for variable in ${!APPTAINERENV_@} ${!SINGULARITYENV_@}; do unset "$variable"; done
repo="${GAMMABOARD_SOURCE_REPOSITORY:?repository required}"
sha="${GAMMABOARD_SOURCE_REVISION:?resolved source revision required}"
[[ "$sha" =~ ^[0-9a-f]{40}$ ]]
mkdir -p "${workspace}/artifacts/download-cache" "${workspace}/images/${family}"
source_dir="${build_dir}/src"
git init "$source_dir"
git -C "$source_dir" remote add origin "$repo"
git -C "$source_dir" fetch --depth 1 origin "$sha"
git -C "$source_dir" checkout --detach FETCH_HEAD
[[ "$(git -C "$source_dir" rev-parse HEAD)" == "$sha" ]]
[[ -z "$(git -C "$source_dir" status --porcelain)" ]]
python3 "${recipe}/ops/build/render.py" "$recipe" "$build_dir" "$family" "$sha" "${SLURM_CPUS_PER_TASK:-4}"
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
python3 "${recipe}/ops/build/publish.py" "$workspace" "$build_dir" "$family" "$sha"
