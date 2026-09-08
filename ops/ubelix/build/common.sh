#!/usr/bin/env bash

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
  printf '%s\n' "$(($2 - $1))"
}
