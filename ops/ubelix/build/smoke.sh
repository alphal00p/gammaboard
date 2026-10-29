#!/usr/bin/env bash
set -euo pipefail
# Optional paths also let this test exercise a native build without a container.
cd "${1:-/smoke}"
smoke_root="$(pwd -P)"
export LANG=C.UTF-8 LC_ALL=C.UTF-8
phase='configuration'
deploy_pid=''
rm -f smoke.result
board() { gammaboard --runtime-config "${smoke_root}/runtime.toml" "$@"; }
http() { curl --noproxy 127.0.0.1 --connect-timeout 2 --max-time 5 -fsS "$@"; }
step() { phase="$1"; printf 'Smoke: %s\n' "$phase"; }
cleanup() {
    status=$?
    trap - EXIT
    set +e
    if [[ -n "$deploy_pid" ]]; then
        kill -TERM "$deploy_pid" 2>/dev/null
        wait "$deploy_pid"
        deploy_status=$?
        if [[ "$status" == 0 && "$deploy_status" != 0 ]]; then
            status=$deploy_status
            phase='service cleanup'
        fi
    fi
    board db stop > cleanup.log 2>&1
    cleanup_status=$?
    if [[ "$status" == 0 && "$cleanup_status" != 0 ]]; then
        status=$cleanup_status
        phase='database cleanup'
    fi
    if [[ "$status" == 0 ]]; then
        printf 'passed\n' > smoke.result
        printf 'Smoke: passed and stopped\n'
    else
        printf 'Smoke test failed during %s (exit %s). Diagnostics:\n' "$phase" "$status" >&2
        for log in deploy.log resources/db/logfile workers.json nodes.json assignment.json run-result.json cleanup.log resources/logs/nodes/*.stderr.log; do
            if [[ -f "$log" ]]; then
                printf '\n%s\n' "$log" >&2
                tail -n 60 "$log" >&2
            fi
        done
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
python3 - "${2:-/usr/local/share/gammaboard/dashboard}" <<'PY'
import json
import socket
import sys
from pathlib import Path
listeners = []
for _ in range(3):
    s = socket.socket()
    s.bind(('127.0.0.1', 0))
    listeners.append(s)
pg, api, web = [s.getsockname()[1] for s in listeners]
Path('ports').write_text(f'{pg} {api} {web}\n')
Path('runtime.toml').write_text(f'''[database]
url="postgresql://postgres@127.0.0.1:{pg}/gammaboard_smoke"
[resources]
roots=[{json.dumps(str(Path.cwd() / 'resources'))}]
[local_postgres]
max_connections=64
''')
Path('server.toml').write_text(f'''api_host="127.0.0.1"
api_port={api}
allow_local_node_spawn=true
[frontend]
host="127.0.0.1"
port={web}
build_dir={json.dumps(sys.argv[1])}
''')
PY
read -r pg api web < ports
step 'starting services'
# Launch the executable directly so $! identifies the supervisor, not a function's shell.
gammaboard --runtime-config "${smoke_root}/runtime.toml" deploy --server-config "${smoke_root}/server.toml" > deploy.log 2>&1 &
deploy_pid=$!
wait_http() {
    local deadline=$((SECONDS + 90))
    while (( SECONDS < deadline )); do
        kill -0 "$deploy_pid"
        if http "$1" > "$2" 2>/dev/null; then return 0; fi
        sleep 0.5
    done
    return 1
}
wait_http "http://127.0.0.1:${api}/api/health" health.json
step 'waiting for frontend'
wait_http "http://127.0.0.1:${web}" frontend.html
cat > run.toml <<'TOML'
name="image-smoke"
[evaluator]
kind="unit"
continuous_dims=1
[[task_queue]]
kind="sample"
stop_condition={max_samples=32}
accumulator={config="scalar"}
sampler={config={kind="naive_monte_carlo"}}
TOML
step 'creating run and launching workers'
board run create run.toml
http -H 'Content-Type: application/json' -d '{"count":2,"name_prefix":"smoke"}' "http://127.0.0.1:${api}/api/node-launch-requests" > workers.json
step 'waiting for worker registration'
deadline=$((SECONDS + 60))
while true; do
    kill -0 "$deploy_pid"
    http "http://127.0.0.1:${api}/api/nodes" > nodes.json
    if python3 -c 'import json,sys; nodes=json.load(open("nodes.json")); sys.exit(0 if sum(n["status"] == "idle" for n in nodes) == 2 else 1)'; then break; fi
    (( SECONDS < deadline ))
    sleep 0.5
done
step 'assigning sampler and evaluator'
board --json node auto-assign image-smoke --max-evaluators 1 > assignment.json
python3 - <<'PY'
import json
assignment = json.load(open('assignment.json'))
if len(assignment['assigned_samplers']) != 1 or len(assignment['assigned_evaluators']) != 1:
    raise SystemExit(f'expected one sampler and one evaluator; got {assignment}')
PY
step 'waiting for 32 completed samples'
board run wait image-smoke --until completed --timeout 60s
board --json run inspect image-smoke > run-result.json
python3 - <<'PY'
import json
result = json.load(open('run-result.json'))
if len(result['tasks']) != 1 or result['tasks'][0]['state'] != 'completed' or result['tasks'][0]['nr_completed_samples'] != 32:
    raise SystemExit('smoke run did not complete exactly 32 samples; see run-result.json')
PY
step 'stopping services'
