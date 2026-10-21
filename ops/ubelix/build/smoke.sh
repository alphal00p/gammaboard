#!/usr/bin/env bash
set -euo pipefail
cd /smoke
python3 - <<'PY'
import socket
from pathlib import Path
listeners=[]
for _ in range(3):
 s=socket.socket();s.bind(('127.0.0.1',0));listeners.append(s)
pg,api,web=[s.getsockname()[1] for s in listeners]
Path('ports').write_text(f'{pg} {api} {web}\n')
Path('runtime.toml').write_text(f'''[database]
url="postgresql://postgres@127.0.0.1:{pg}/gammaboard_smoke"
[resources]
roots=["/smoke/resources"]
[local_postgres]
max_connections=64
''')
Path('server.toml').write_text(f'''api_port={api}
[frontend]
host="127.0.0.1"
port={web}
build_dir="/usr/local/share/gammaboard/dashboard"
''')
PY
read -r pg api web < ports
board() { gammaboard --runtime-config /smoke/runtime.toml "$@"; }
deploy_pid=''
cleanup() {
    if [[ -n "$deploy_pid" ]]; then kill -TERM "$deploy_pid" 2>/dev/null || true; wait "$deploy_pid" || true; fi
    board db stop || true
}
trap cleanup EXIT
board deploy --server-config /smoke/server.toml > deploy.log 2>&1 &
deploy_pid=$!
ready=0
for _ in $(seq 1 90); do
    kill -0 "$deploy_pid"
    if curl -fsS "http://127.0.0.1:${api}/api/health" > health.json; then ready=1; break; fi
    sleep 1
done
[[ "$ready" == 1 ]]
curl -fsS "http://127.0.0.1:${web}" > frontend.html
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
board run create run.toml
board node start-local 2
board node auto-assign image-smoke
for _ in $(seq 1 60); do
    result="$(psql "postgresql://postgres@127.0.0.1:${pg}/gammaboard_smoke" -Atc "SELECT state || ':' || nr_completed_samples FROM run_tasks")"
    if [[ "$result" == completed:32 ]]; then printf 'passed\n' > smoke.result; exit 0; fi
    if [[ "$result" == failed:* ]]; then exit 1; fi
    sleep 1
done
exit 1
