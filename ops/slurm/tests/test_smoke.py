"""Exercise the actual smoke script with delayed registration and process cleanup."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture
def smoke(tmp_path):
    binaries = tmp_path / "bin"
    binaries.mkdir()
    (binaries / "python3").symlink_to(sys.executable)
    stub = binaries / "gammaboard"
    stub.write_text(r"""#!/usr/bin/env python3
import json,os,signal,sys,time,tomllib
from pathlib import Path
from urllib.parse import urlsplit
root=Path.cwd()
mode=os.environ.get('SMOKE_TEST_MODE','success')
if Path(sys.argv[0]).name=='curl':
    path=urlsplit(sys.argv[-1]).path or '/'
    if path=='/api/health':
        if not (root/'supervisor.pid').exists():sys.exit(7)
        print('{"status":"ok"}')
    elif path=='/':print('<html>dashboard</html>')
    else:raise AssertionError(sys.argv)
    sys.exit(0)
args=sys.argv[3:]
args=[a for a in args if a!='--json']
if args[0]=='start':
    if mode=='startup_failure':
        print('injected startup failure',flush=True);sys.exit(23)
    stopped=False
    def stop(*_):
        global stopped
        stopped=True
    signal.signal(signal.SIGTERM,stop)
    (root/'supervisor.pid').write_text(str(os.getpid()))
    while not stopped:time.sleep(.02)
    (root/'supervisor-stopped').write_text('yes')
elif args[:3]==['local','run','validate']:
    card=tomllib.loads((root/'run.toml').read_text())
    assert card['evaluator']['kind']=='symbolica' and '--probe' in args
    if mode=='missing_compiler':
        print('Symbolica expression compilation with g++ failed',file=sys.stderr);sys.exit(1)
    print('{"runtime_probes":1}')
elif args[:2]==['worker','launch']:
    assert '--wait' in args and '--count' in args and '--timeout' in args
    if mode=='registration_failure':sys.exit(1)
    (root/'workers-ready').write_text('yes')
    print('{"request_ids":["1"]}')
elif args[:3]==['config','reload','--origins']:print('{"allow_loopback_origins":true}')
elif args[:2]==['run','create']:print('created run_id=1 name=image-smoke')
elif args[:2]==['run','start']:
    ready=(root/'workers-ready').exists()
    assignment={'assigned_samplers':['smoke-1'] if ready and mode!='empty_assignment' else [],'assigned_evaluators':['smoke-2'] if ready and mode!='empty_assignment' else []}
    assert ready, 'start must follow worker registration'
    print(json.dumps({'start':{'state':'started','assignments':assignment['assigned_samplers']+assignment['assigned_evaluators']}}))
elif args[:2]==['run','wait']:
    assert len(json.loads((root/'assignment.json').read_text())['start']['assignments'])==2
    print('completed')
elif args[:2]==['run','inspect']:
    print(json.dumps({'tasks':[{'state':'completed','nr_completed_samples':32}]}))
elif args[:3]==['local','db','stop']:(root/'db-stopped').write_text('yes')
else:raise AssertionError(args)
""")
    stub.chmod(0o755)
    (binaries / "curl").symlink_to(stub)
    (binaries / "gb").symlink_to(stub)
    work = tmp_path / "smoke"
    work.mkdir()
    env = dict(os.environ, PATH=f"{binaries}:{os.environ['PATH']}")
    yield work, env
    pid_file = work / "supervisor.pid"
    if pid_file.exists():
        pid = int(pid_file.read_text())
        try:
            if str(work).encode() in Path(f"/proc/{pid}/cmdline").read_bytes():
                os.kill(pid, signal.SIGTERM)
        except FileNotFoundError:
            pass


@pytest.mark.parametrize("mode", ["success", "empty_assignment", "startup_failure", "missing_compiler", "registration_failure"])
def test_smoke_waits_for_registration_reports_failures_and_stops_supervisor(smoke, mode):
    work, env = smoke
    # A stale marker must never make a failed check look successful.
    (work / "smoke.result").write_text("passed\n")
    result = subprocess.run(
        ["bash", str(ROOT / "jobs/smoke.sh"), str(work), "/test/frontend"],
        env=dict(env, SMOKE_TEST_MODE=mode),
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert (work / "db-stopped").exists(), result.stderr
    if mode != "startup_failure":
        pid = int((work / "supervisor.pid").read_text())
        assert (work / "supervisor-stopped").exists(), result.stderr
        with pytest.raises(ProcessLookupError):
            os.kill(pid, 0)
        if mode not in ("missing_compiler", "registration_failure"):
            assert (work / "workers-ready").exists()
    if mode == "success":
        assert result.returncode == 0, result.stderr
        assert (work / "smoke.result").read_text().strip() == "passed"
        assert (
            json.loads((work / "run-result.json").read_text())["tasks"][0]["nr_completed_samples"]
            == 32
        )
    else:
        assert result.returncode != 0
        assert not (work / "smoke.result").exists()
        expected = {
            "registration_failure": "waiting for worker registration",
            "empty_assignment": "assigning sampler and evaluator",
            "startup_failure": "starting services",
            "missing_compiler": "validating Symbolica runtime",
        }[mode]
        assert f"Smoke test failed during {expected}" in result.stderr
        if mode == "startup_failure":
            assert "injected startup failure" in result.stderr
