#!/usr/bin/env bash
set -euo pipefail

host="${1:-ubelix}"
remote_folder="${2:-gammaboard}"

if (($# > 2)) || [[ ! "$remote_folder" =~ ^[A-Za-z0-9._/-]+$ || "$remote_folder" = /* || "$remote_folder" == *..* || "$remote_folder" == *//* ]]; then
    echo "usage: ops/ubelix/sync_ops.sh [host] [safe-relative-remote-folder]" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
remote_root="/storage/research/itp_localunitaritydata/$remote_folder"
cd "$repo_root"

ssh "$host" "mkdir -p '$remote_root/ops/build' '$remote_root/ops/config' '$remote_root/ops/slurm' '$remote_root/resources/templates' '$remote_root/docs'"
scp -r ops/ubelix/build/* "$host:$remote_root/ops/build/"
scp -r ops/ubelix/config/* "$host:$remote_root/ops/config/"
scp -r ops/ubelix/slurm/* "$host:$remote_root/ops/slurm/"
# Shared run/task cards are maintained once; only node allocation is cluster-specific.
scp -r resources/templates/* "$host:$remote_root/resources/templates/"
scp -r ops/ubelix/resources/templates/nodes "$host:$remote_root/resources/templates/"
scp -r docs/* "$host:$remote_root/docs/"
scp ops/ubelix/ubelix.py "$host:$remote_root/ubelix.py"
scp ops/ubelix/README.md "$host:$remote_root/README.md"
