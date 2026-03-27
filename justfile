bin := "./target/dev-optim/gammaboard"

build-backend:
    cargo build --profile dev-optim

build:
    just build-frontend
    just build-backend

build-frontend:
    cd dashboard && npm run build

serve-backend:
    {{bin}} server

serve-frontend:
    cd dashboard && npx serve build

deploy-local-prod:
    #!/usr/bin/env bash
    set -euo pipefail

    just build
    mkdir -p logs tmp/nginx/client_body tmp/nginx/proxy tmp/nginx/fastcgi tmp/nginx/uwsgi tmp/nginx/scgi

    {{bin}} server --server-config configs/server/local-prod.toml &
    backend_pid=$!
    echo "$backend_pid" > "$PWD/logs/local-prod-backend.pid"

    cleanup() {
        kill "$backend_pid" >/dev/null 2>&1 || true
        wait "$backend_pid" >/dev/null 2>&1 || true
        rm -f "$PWD/logs/local-prod-backend.pid"
    }
    trap cleanup EXIT INT TERM

    echo "Local prod stack is up"
    echo "Open: http://localhost:8080"
    nginx -e "$PWD/logs/nginx-local-prod-error.log" -p "$PWD" -c "$PWD/configs/nginx/local-prod.conf" -g 'daemon off;'

stop-local-prod:
    #!/usr/bin/env bash
    set -euo pipefail

    backend_pid_file="$PWD/logs/local-prod-backend.pid"
    nginx_pid_file="$PWD/logs/nginx-local-prod.pid"

    if [[ -f "$nginx_pid_file" ]]; then
        nginx_pid=$(cat "$nginx_pid_file")
        if kill -0 "$nginx_pid" >/dev/null 2>&1; then
            kill "$nginx_pid"
            echo "stopped local nginx (pid=$nginx_pid)"
        fi
        rm -f "$nginx_pid_file"
    else
        echo "local nginx already stopped"
    fi

    if [[ -f "$backend_pid_file" ]]; then
        backend_pid=$(cat "$backend_pid_file")
        if kill -0 "$backend_pid" >/dev/null 2>&1; then
            kill "$backend_pid"
            echo "stopped local backend (pid=$backend_pid)"
        fi
        rm -f "$backend_pid_file"
    else
        echo "local backend already stopped"
    fi

test-e2e:
    just stop-kill
    cargo test -q --test full_stack_cli -- --ignored --nocapture

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    run_live_test="live-test"
    run_symbolica_poly="symbolica-poly-test"
    run_symbolica_sin="symbolica-sin-test"

    {{bin}} db delete --yes
    {{bin}} db start
    {{bin}} node auto-run 8

    sleep 4

    {{bin}} run add "configs/runs/live-test-unit-naive-scalar.toml"
    {{bin}} run add "configs/runs/symbolica-live-test.toml"
    {{bin}} run add "configs/runs/symbolica-live-test-sin.toml"

    {{bin}} node assign "w-1" evaluator "$run_live_test"
    {{bin}} node assign "w-2" sampler-aggregator "$run_live_test"

    {{bin}} auto-assign "$run_symbolica_poly" 5

    echo "initial assignments settled"
    sleep 10

    echo "move two workers from run 2 to run 3"
    {{bin}} node assign "w-3" sampler-aggregator "$run_symbolica_sin"
    {{bin}} node assign "w-8" evaluator "$run_symbolica_sin"

    sleep 10

    echo "pause run 3 and return workers to run 2"
    {{bin}} run pause "$run_symbolica_sin"
    sleep 6
    {{bin}} auto-assign "$run_symbolica_poly" 1

    sleep 10

    echo "pause run 2 and resume run 3 with all symbolica workers"
    {{bin}} run pause "$run_symbolica_poly"
    sleep 6
    {{bin}} auto-assign "$run_symbolica_sin" 5

stop:
    -{{bin}} run pause -a
    -{{bin}} node stop -a
    -@stty sane

stop-kill:
    just stop
    -pkill -f "{{bin}} node run"
    -pkill -f "{{bin}} server"
    -@stty sane
