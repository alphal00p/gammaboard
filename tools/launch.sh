#!/usr/bin/env bash
# Development launcher shared by the lightweight client and service executable.
set -euo pipefail
family="$1"
shift
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
caller_dir="$PWD"
target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "$target_root" != /* ]]; then target_root="${repo_root}/${target_root}"; fi
export CARGO_TARGET_DIR="$target_root"
profile="${GAMMABOARD_PROFILE:-dev-optim}"
case "$profile" in
    debug) build_args=(); directory=debug ;;
    release) build_args=(--release); directory=release ;;
    dev-optim) build_args=(--profile dev-optim); directory=dev-optim ;;
    *) echo "unsupported GAMMABOARD_PROFILE=$profile" >&2; exit 2 ;;
esac
bin="${target_root}/${directory}/${family}"
instance="${caller_dir}/gammaboard.toml"
allow=false
next=false
for arg in "$@"; do
    if [[ "$next" == true ]]; then instance="$arg"; next=false; continue; fi
    case "$arg" in
        --instance) next=true ;;
        --instance=*) instance="${arg#--instance=}" ;;
        --allow-local-processes) allow=true ;;
    esac
done
if [[ "$instance" != /* ]]; then instance="${caller_dir}/${instance}"; fi
managed=false
if [[ -f "$instance" ]]; then
    managed="$(python3 -c 'import sys,tomllib; v=tomllib.load(open(sys.argv[1],"rb")).get("externally_managed",False); assert isinstance(v,bool); print(str(v).lower())' "$instance")"
fi
if [[ "$managed" == true && "$allow" == false ]]; then
    if [[ ! -x "$bin" ]]; then
        echo "This instance is externally managed. Use a prepared executable; compiling here requires --allow-local-processes." >&2
        exit 2
    fi
else
    package=gammaboard
    if [[ "$family" == gb ]]; then package=gammaboard-client; fi
    (
        cd "$repo_root"
        cargo build --package "$package" --bin "$family" "${build_args[@]}"
    )
fi
if [[ "$family" == gb ]]; then
    export GAMMABOARD_SERVICE_BINARY="${GAMMABOARD_SERVICE_BINARY:-${repo_root}/gammaboard}"
fi
exec "$bin" "$@"
