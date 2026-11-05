#!/usr/bin/env python3
"""Render cached base environments and an application build from frozen sources."""

import hashlib
import json
from pathlib import Path
import re
import shutil
import sys

recipe, build, family, sha, cpus, scratch, image_cache = sys.argv[1:]
recipe, build, scratch, image_cache = map(Path, (recipe, build, scratch, image_cache))
if int(cpus) < 1:
    raise ValueError("build CPU count must be positive")
pins = json.loads((recipe / "build/images.lock.json").read_text())
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
}


def expand(text, replacements):
    for key, value in replacements.items():
        text = text.replace("@" + key + "@", value)
    return text


prepared = {}
for role in ("builder", "runtime"):
    text = expand((recipe / f"build/{role}-base.def").read_text(), values)
    key = hashlib.sha256(text.encode()).hexdigest()
    prepared[role] = {"recipe_sha256": key, "path": str(image_cache / f"{role}-{key}.sif")}
    (build / f"{role}-base.def").write_text(text)

template = (recipe / "build/service.def").read_text()
# Only the builder environment and application build commands affect Cargo reuse.
# Runtime packages, final-image layout, source revisions, paths and CPU counts do not.
builder_stage = template.split("\nBootstrap:", 1)[0]
cache_key = hashlib.sha256(
    (prepared["builder"]["recipe_sha256"] + "\0" + builder_stage).encode()
).hexdigest()
values.update({
    "BUILDER_IMAGE": prepared["builder"]["path"],
    "RUNTIME_IMAGE": prepared["runtime"]["path"],
    "SOURCE": json.dumps(str(scratch / "src")),
    "REVISION": sha,
    "CPUS": str(int(cpus)),
    "FAMILY": family,
    "CACHE_KEY": cache_key,
    "CARGO_BUILD": "cargo build --release --locked --bin gammaboard"
    if family == "gammaboard"
    else "cargo build --release --locked -p gammaloop-api --bin gammaloop --features ufo_support",
})
(build / "service.def").write_text(expand(template, values))
(build / "prepared-images.json").write_text(json.dumps(prepared, indent=2) + "\n")
shutil.copy2(recipe / "build/images.lock.json", build / "images.lock.json")
shutil.copy2(recipe / "build/smoke.sh", build / "smoke.sh")
