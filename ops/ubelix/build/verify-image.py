#!/usr/bin/env python3
import json
from pathlib import Path
import subprocess
import sys
workspace = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(workspace))
from gbops.deployment import Deployments
image = Deployments(workspace, 0).selected_image(sys.argv[2] if len(sys.argv) > 2 else None)
actual = json.loads(subprocess.check_output(['apptainer','exec','--cleanenv',image['path'],'gammaboard','build-info'],text=True))
if actual != image['build_info']: raise SystemExit('image build identity mismatch')
print(json.dumps(actual,indent=2))
