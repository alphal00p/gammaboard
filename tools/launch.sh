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
require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing dependency: $1. $2" >&2
        exit 2
    fi
}
managed=false
if [[ -f "$instance" ]]; then
    require_tool python3 "Install Python 3.11 or newer and make python3 available on PATH, or use a prepared gb binary."
    managed="$(python3 - "$instance" <<'PYTHON'
import sys
try:
    import tomllib
except ImportError:
    sys.exit("Missing dependency: Python 3.11 or newer is required by the repository launcher (tomllib unavailable).")
try:
    with open(sys.argv[1], "rb") as source:
        value = tomllib.load(source).get("externally_managed", False)
    if not isinstance(value, bool):
        raise ValueError("externally_managed must be true or false")
except (OSError, ValueError) as error:
    sys.exit(f"Cannot read instance configuration {sys.argv[1]}: {error}")
print(str(value).lower())
PYTHON
)"
fi
if [[ "$managed" == true && "$allow" == false ]]; then
    if [[ ! -x "$bin" ]]; then
        echo "This instance is externally managed. Use a prepared executable; compiling here requires --allow-local-processes." >&2
        exit 2
    fi
else
    require_tool cargo "Install the Rust toolchain and make cargo available on PATH, or use a prepared gb binary."
    require_tool rustc "Install the Rust toolchain and make rustc available on PATH."
    if [[ -f "$instance" ]]; then
        python3 "$repo_root/tools/frontend.py" "$instance" "$@"
    fi
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
