#!/usr/bin/env python3
"""Render a standalone definition with immutable base images and clean sources."""

import json
from pathlib import Path
import re
import shutil
import sys

workspace, build, family, sha, cpus = sys.argv[1:]
workspace, build = Path(workspace), Path(build)
pins = json.loads((workspace / "build/images.lock.json").read_text())
for item in pins.values():
    repository, separator, checksum = item["image"].partition("@sha256:")
    if (
        not separator
        or ":" in repository.rsplit("/", 1)[-1]
        or not re.fullmatch(r"[0-9a-f]{64}", checksum)
    ):
        raise ValueError(
            "base image must use repository@sha256:<64 hex digits>, without a tag (Apptainer requirement)"
        )
values = {
    "NODE_IMAGE": pins["node"]["image"],
    "RUST_IMAGE": pins["rust"]["image"],
    "DEBIAN_IMAGE": pins["debian"]["image"],
    "SOURCE": json.dumps(str(build / "src")),
    "REVISION": sha,
    "CPUS": str(int(cpus)),
    "FAMILY": family,
    "CARGO_BUILD": "cargo build --release --locked --bin gammaboard"
    if family == "gammaboard"
    else "cargo build --release --locked -p gammaloop-api --bin gammaloop --features ufo_support",
}
text = (workspace / "build/service.def").read_text()
for key, value in values.items():
    text = text.replace("@" + key + "@", value)
(build / "service.def").write_text(text)
shutil.copy2(workspace / "build/images.lock.json", build / "images.lock.json")
shutil.copy2(workspace / "build/smoke.sh", build / "smoke.sh")
