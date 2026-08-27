#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

compiler="${CXX:-c++}"
if ! command -v "$compiler" >/dev/null 2>&1; then
    echo "missing C++ compiler '$compiler'; run 'nix develop' or set CXX" >&2
    exit 127
fi

mkdir -p resources/artifacts
"$compiler" \
    -shared -O3 -fPIC -ffast-math -funsafe-math-optimizations \
    -o resources/artifacts/variable_theta.so \
    examples/symbolica-variable-theta/variable_theta.cpp
