#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root/process_api/examples/rust_breit_wigner_evaluator"

apptainer build runtime.sif apptainer.def
