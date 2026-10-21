#!/usr/bin/env bash
set -euo pipefail
host="${1:-ubelix}"
remote_folder="${2:-gammaboard}"
if (($# > 2)) || [[ ! "$remote_folder" =~ ^[A-Za-z0-9._/-]+$ || "$remote_folder" = /* || "$remote_folder" =~ (^|/)\.(/|$) || "$remote_folder" == *..* || "$remote_folder" == *//* ]]; then
    echo 'usage: ops/ubelix/sync_ops.sh [host] [safe-relative-remote-folder]' >&2
    exit 2
fi
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
remote_root="/storage/research/itp_localunitaritydata/$remote_folder"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/ops" "$staging/templates"
cp -a "$repo_root/ops/ubelix/build" "$repo_root/ops/ubelix/config" "$repo_root/ops/ubelix/slurm" "$staging/ops/"
# Assemble the complete template set before mirroring, including cluster node cards.
cp -a "$repo_root/resources/templates/." "$staging/templates/"
cp -a "$repo_root/ops/ubelix/resources/templates/nodes/." "$staging/templates/nodes/"
ssh "$host" "for directory in '$remote_root/ops' '$remote_root/gbops' '$remote_root/docs' '$remote_root/resources/templates'; do [ ! -L \"\$directory\" ] || { echo 'managed directory is a symlink' >&2; exit 1; }; mkdir -p \"\$directory\"; done"
# Deletion is deliberately limited to these repository-owned subtrees.
rsync -rlt --delete-delay -- "$staging/ops/" "$host:$remote_root/ops/"
rsync -rlt --delete-delay --exclude='__pycache__/' -- "$repo_root/ops/ubelix/gbops/" "$host:$remote_root/gbops/"
rsync -rlt --delete-delay -- "$repo_root/docs/" "$host:$remote_root/docs/"
rsync -rlt --delete-delay -- "$staging/templates/" "$host:$remote_root/resources/templates/"
# These two managed entry points are files, not mirrors of the workspace root.
rsync -lt -- "$repo_root/ops/ubelix/ubelix.py" "$host:$remote_root/ubelix.py"
rsync -lt -- "$repo_root/ops/ubelix/README.md" "$host:$remote_root/README.md"
