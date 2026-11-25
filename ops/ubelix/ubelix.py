#!/usr/bin/env python3
"""Compatibility entry point; use ops/slurm/slurm.py for new deployments."""
import runpy
import sys
from pathlib import Path
root=Path(__file__).resolve().parents[1]/"slurm"
sys.path.insert(0,str(root))
runpy.run_path(str(root/"slurm.py"),run_name="__main__")
