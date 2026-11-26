import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

FIXTURE = """
import os,sys,time
import numpy as np
from gammaboard_process import *
from gammaboard_process.evaluation import DIAGNOSTIC_COUNTS,DIAGNOSTIC_TIMES
class Provider(Evaluator):
 def __init__(self,*,domain=None,mode="good"):
  self.mode=mode
  self.domain=DiscreteDomain((DomainBranch(2,RectangularDomain((),1)),DomainBranch(5,RectangularDomain((),2))))
 def descriptor(self):return EvaluatorDescriptor(self.domain,["constant","linear"],details=True,diagnostics=True)
 def evaluate(self,discrete,continuous,request):
  n=len(continuous)
  shift=n if self.mode=="batch_bias" else os.getpid() if self.mode=="process_bias" else 0
  values={"constant":np.full(n,2.+shift),"linear":np.asarray([sum(x) for x in continuous])}
  return EvaluationBatch(values,
   details=[dict(integrand_value_re=2.,integrand_value_im=0.,parameterization_jacobian=None) for _ in range(n)] if request.details else None,
   diagnostics=[dict.fromkeys(DIAGNOSTIC_COUNTS,0)|dict.fromkeys(DIAGNOSTIC_TIMES,0.)|dict(count_total=1,total_eval_time_ms=time.monotonic()) for _ in range(n)] if request.diagnostics else None)
run_evaluator(Provider)
"""


def invoke(tmp_path, mode):
    worker = tmp_path / "worker.py"
    worker.write_text(FIXTURE)
    points = tmp_path / "points.json"
    points.write_text(
        json.dumps(
            [
                dict(discrete=[2], continuous=[0.2]),
                dict(discrete=[5], continuous=[0.3, 0.4]),
                dict(discrete=[2], continuous=[0.8]),
            ]
        )
    )
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "process_api/check_evaluator.py"),
            "--command",
            json.dumps([sys.executable, str(worker)]),
            "--init-args",
            json.dumps(dict(mode=mode)),
            "--points",
            str(points),
            "--json",
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return result, json.loads(result.stdout)


def test_checker_exercises_ragged_constant_sidecars_encodings_and_replay(tmp_path):
    result, report = invoke(tmp_path, "good")
    assert result.returncode == 0, (result.stderr, report)
    checks = {c["check"]: c["status"] for c in report["checks"]}
    assert checks["JSON/binary equivalence"] == "pass"
    assert checks["fresh-process replay"] == "pass"
    assert checks["reference outputs"] == "skip"
    assert checks["components ['linear', 'constant']"] == "pass"
    assert checks["invalid continuous nan and recovery"] == "pass"


def test_checker_rejects_batch_dependent_values(tmp_path):
    result, report = invoke(tmp_path, "batch_bias")
    assert result.returncode == 1
    assert report["status"] == "failed"


def test_checker_rejects_process_dependent_values(tmp_path):
    result, report = invoke(tmp_path, "process_bias")
    assert result.returncode == 1
    assert report["status"] == "failed"
