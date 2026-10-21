#!/usr/bin/env bash

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

git_resolve_sha() {
  local repo_url="$1"
  local revision="${2:-HEAD}"
  local sha
  if [[ "${revision}" =~ ^[0-9a-fA-F]{40}$ ]]; then
    printf '%s\n' "${revision,,}"
    return
  fi
  sha="$(git ls-remote "${repo_url}" "${revision}" "${revision}^{}" | awk '
    NR == 1 { first = $1 }
    $2 ~ /\^\{\}$/ { peeled = $1 }
    END { print peeled != "" ? peeled : first }
  ')"
  if [[ -z "${sha}" ]]; then
    echo "failed to resolve revision '${revision}' for ${repo_url}" >&2
    exit 1
  fi
  printf '%s\n' "${sha}"
}
