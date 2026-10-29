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
import json,os,signal,sys,time
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
    elif path=='/api/node-launch-requests':print('{"requested":2,"started":2}')
    elif path=='/api/nodes':
        counter=root/'polls'
        n=int(counter.read_text())+1 if counter.exists() else 1
        counter.write_text(str(n))
        print(json.dumps([{'node_name':f'smoke-{i}','status':'idle'} for i in range(min(n-1,2))]))
    else:raise AssertionError(sys.argv)
    sys.exit(0)
args=sys.argv[3:]
args=[a for a in args if a!='--json']
if args[0]=='deploy':
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
elif args[:2]==['run','create']:print('created run_id=1 name=image-smoke')
elif args[:2]==['node','auto-assign']:
    n=int((root/'polls').read_text())
    assignment={'assigned_samplers':['smoke-1'] if n>=3 and mode!='empty_assignment' else [],'assigned_evaluators':['smoke-2'] if n>=3 and mode!='empty_assignment' else []}
    (root/'assigned-after-polls').write_text(str(n))
    print(json.dumps(assignment))
elif args[:2]==['run','wait']:
    assert len(json.loads((root/'assignment.json').read_text())['assigned_samplers'])==1
    print('completed')
elif args[:2]==['run','inspect']:
    print(json.dumps({'tasks':[{'state':'completed','nr_completed_samples':32}]}))
elif args[:2]==['db','stop']:(root/'db-stopped').write_text('yes')
else:raise AssertionError(args)
""")
    stub.chmod(0o755)
    (binaries / "curl").symlink_to(stub)
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


@pytest.mark.parametrize("mode", ["success", "empty_assignment", "startup_failure"])
def test_smoke_waits_for_registration_reports_failures_and_stops_supervisor(smoke, mode):
    work, env = smoke
    # A stale marker must never make a failed check look successful.
    (work / "smoke.result").write_text("passed\n")
    result = subprocess.run(
        ["bash", str(ROOT / "build/smoke.sh"), str(work), "/test/frontend"],
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
        assert int((work / "assigned-after-polls").read_text()) >= 3
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
        expected = (
            "assigning sampler and evaluator" if mode == "empty_assignment" else "starting services"
        )
        assert f"Smoke test failed during {expected}" in result.stderr
        if mode == "startup_failure":
            assert "injected startup failure" in result.stderr
