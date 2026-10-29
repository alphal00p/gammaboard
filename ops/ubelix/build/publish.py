#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

workspace, build, family, revision = sys.argv[1:]
workspace, build = Path(workspace), Path(build)
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from gbops.deployment import Deployments, digest, write_json

if (build / "smoke.result").read_text().strip() != "passed":
    raise ValueError("image smoke validation did not pass")
info = json.loads((build / "build-info.json").read_text())
if family == "gammaboard" and info["git_revision"] != revision:
    raise ValueError("built executable provenance differs from the requested source")
image = build / "image.sif"
checksum = digest(image)
directory = workspace / "images" / family / revision / checksum
with Deployments(workspace, 0).locked():
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / (family + ".sif")
    # link is atomic and refuses to overwrite a published image.
    try:
        os.link(image, target)
    except FileExistsError:
        if digest(target) != checksum:
            raise ValueError("published digest path is corrupt")
    target.chmod(0o444)
    manifest = {
        "path": str(target),
        "sha256": checksum,
        "source_revision": revision,
        "family": family,
        "build_info": info,
        "base_images": json.loads((build / "images.lock.json").read_text()),
        "build_directory": str(build),
        "validated": True,
    }
    write_json(directory / "image.json", manifest)
    write_json(build / "published.json", manifest)
print(json.dumps(manifest, indent=2))
