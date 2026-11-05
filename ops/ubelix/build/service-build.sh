#!/usr/bin/env bash
set -euo pipefail
umask 077
family="${1:?service family required}"
case "$family" in gammaboard|gammaloop) ;; *) exit 2 ;; esac
workspace="${GAMMABOARD_WORKSPACE_ROOT:?workspace from build record required}"
recipe="${GAMMABOARD_RECIPE_ROOT:?frozen recipe required}"
build_dir="${GAMMABOARD_BUILD_DIR:?private build directory required}"
scratch="${GAMMABOARD_BUILD_SCRATCH:?job-local build directory required}"
for tool in apptainer python3; do
    command -v "$tool" >/dev/null || { echo "missing required command: $tool" >&2; exit 1; }
done
# Explicit container environment overrides bypass --cleanenv. Do not inherit
# runtime settings (including deployment identity) into release validation.
for variable in ${!APPTAINERENV_@} ${!SINGULARITYENV_@}; do unset "$variable"; done
sha="${GAMMABOARD_SOURCE_REVISION:?checked-out source revision required}"
cpus="${SLURM_CPUS_PER_TASK:-4}"
image_cache="${workspace}/artifacts/image-cache"
mkdir -p "${workspace}/artifacts/download-cache" "${workspace}/artifacts/build-cache" "$image_cache"
python3 "${recipe}/build/render.py" "$recipe" "$build_dir" "$family" "$sha" "$cpus" "$scratch" "$image_cache"
# Unpacking and packing touch thousands of files; only caches and completed images
# belong on shared storage. Python owns and cleans this private scratch directory.
export APPTAINER_TMPDIR="${scratch}/tmp"
export APPTAINER_CACHEDIR="${workspace}/artifacts/download-cache/apptainer"
mkdir -p "$APPTAINER_TMPDIR" "$APPTAINER_CACHEDIR"
printf 'Build scratch: %s\n' "$scratch"
printf 'phase\telapsed_seconds\n' > "${build_dir}/timings.tsv"
timed() {
    local phase="$1" started=$SECONDS
    shift
    printf 'Build: %s\n' "$phase"
    "$@"
    printf 'Build timing: %s %ss\n' "$phase" "$((SECONDS - started))"
    printf '%s\t%s\n' "$phase" "$((SECONDS - started))" >> "${build_dir}/timings.tsv"
}
packing=(-comp gzip -Xcompression-level 1 -processors "$cpus")
prepare_base() {
    local role="$1" cached="$2" staged
    if [[ -f "$cached" ]]; then
        printf 'Build: reuse %s\n' "$cached"
        return
    fi
    apptainer build --mksquashfs-args "${packing[*]}" \
        "${scratch}/${role}-base.sif" "${build_dir}/${role}-base.def"
    # Publish only a complete base image. Builds are serialized per workspace.
    staged="$(mktemp "${cached}.tmp.XXXXXX")"
    cp "${scratch}/${role}-base.sif" "$staged"
    chmod 444 "$staged"
    mv "$staged" "$cached"
    rm "${scratch}/${role}-base.sif"
}
base_paths="$(python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); print(p["builder"]["path"]); print(p["runtime"]["path"])' "${build_dir}/prepared-images.json")"
mapfile -t base_images <<< "$base_paths"
timed builder-base prepare_base builder "${base_images[0]}"
timed runtime-base prepare_base runtime "${base_images[1]}"
timed application-image apptainer build --mksquashfs-args "${packing[*]}" \
    --bind "${workspace}/artifacts/download-cache:/cache" \
    --bind "${workspace}/artifacts/build-cache:/build-cache" \
    "${scratch}/image.sif" "${build_dir}/service.def"
timed copy-image cp "${scratch}/image.sif" "${build_dir}/image.sif"
if [[ "$family" == gammaboard ]]; then
    apptainer exec --cleanenv "${build_dir}/image.sif" gammaboard build-info > "${build_dir}/build-info.json"
    timed smoke apptainer exec --cleanenv --bind "${build_dir}:/smoke" "${build_dir}/image.sif" bash /smoke/smoke.sh
else
    timed smoke apptainer exec --cleanenv "${build_dir}/image.sif" gammaloop --help > "${build_dir}/smoke.log"
    printf '{"build_id":"%s"}\n' "$sha" > "${build_dir}/build-info.json"
    printf 'passed\n' > "${build_dir}/smoke.result"
fi
