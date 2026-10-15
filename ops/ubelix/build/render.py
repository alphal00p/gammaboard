#!/usr/bin/env python3
"""Render a standalone definition with immutable base images and clean sources."""
import json
from pathlib import Path
import shutil
import sys

workspace, build, family, sha, cpus = sys.argv[1:]
workspace, build = Path(workspace), Path(build)
pins = json.loads((workspace / 'ops/build/images.lock.json').read_text())
for item in pins.values():
    if '@sha256:' not in item['image']: raise ValueError('base image must be pinned by digest')
values = {'NODE_IMAGE': pins['node']['image'], 'RUST_IMAGE': pins['rust']['image'],
          'DEBIAN_IMAGE': pins['debian']['image'], 'SOURCE': json.dumps(str(build / 'src')),
          'REVISION': sha, 'CPUS': str(int(cpus)), 'FAMILY': family,
          'CARGO_BUILD': 'cargo build --release --locked --bin gammaboard' if family == 'gammaboard'
          else 'cargo build --release --locked -p gammaloop-api --bin gammaloop --features ufo_support'}
text = (workspace / 'ops/build/service.def').read_text()
for key, value in values.items(): text = text.replace('@' + key + '@', value)
(build / 'service.def').write_text(text)
shutil.copy2(workspace / 'ops/build/images.lock.json', build / 'images.lock.json')
shutil.copy2(workspace / 'ops/build/smoke.sh', build / 'smoke.sh')
