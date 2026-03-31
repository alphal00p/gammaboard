bin := "./target/dev-optim/gammaboard"
release_bin := "./target/release/gammaboard"

build-backend:
    cargo build --profile dev-optim

build-backend-release:
    cargo build --release

build:
    just build-frontend
    just build-backend

build-frontend:
    #!/usr/bin/env bash
    set -euo pipefail

    cd dashboard
    if [[ ! -x node_modules/.bin/react-scripts ]]; then
        npm ci
    fi
    npm run build

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

deploy-itphlies-server:
    #!/usr/bin/env bash
    set -euo pipefail

    backend_pid_file="$PWD/logs/itphlies-backend.pid"
    backend_log_file="$PWD/logs/itphlies-backend.log"
    server_pattern="{{release_bin}} server"
    release_worker_pattern="{{release_bin}} node run"
    dev_worker_pattern="{{bin}} node run"

    mkdir -p logs
    if [[ -f "$backend_pid_file" ]]; then
        old_pid=$(cat "$backend_pid_file")
        if kill -0 "$old_pid" >/dev/null 2>&1; then
            kill "$old_pid"
            wait "$old_pid" >/dev/null 2>&1 || true
        fi
    fi

    if [[ -x "{{release_bin}}" ]]; then
        {{release_bin}} run pause -a >/dev/null 2>&1 || true
        {{release_bin}} node stop -a >/dev/null 2>&1 || true
    fi
    if [[ -x "{{bin}}" ]]; then
        {{bin}} run pause -a >/dev/null 2>&1 || true
        {{bin}} node stop -a >/dev/null 2>&1 || true
    fi

    mapfile -t stale_release_worker_pids < <(pgrep -f "$release_worker_pattern" || true)
    for pid in "${stale_release_worker_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
        fi
    done

    mapfile -t stale_dev_worker_pids < <(pgrep -f "$dev_worker_pattern" || true)
    for pid in "${stale_dev_worker_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
        fi
    done

    mapfile -t stale_pids < <(pgrep -f "$server_pattern" || true)
    for pid in "${stale_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
        fi
    done

    {{release_bin}} server --server-config configs/server/itphlies-prod.toml >"$backend_log_file" 2>&1 &
    new_pid=$!
    echo "$new_pid" > "$backend_pid_file"

    echo "Deployed on $(hostname)"
    echo "Backend PID: $new_pid"
    echo "Backend log: $backend_log_file"

deploy-itphlies-nginx:
    #!/usr/bin/env bash
    set -euo pipefail

    mkdir -p logs tmp/nginx/client_body tmp/nginx/proxy tmp/nginx/fastcgi tmp/nginx/uwsgi tmp/nginx/scgi

    nginx_pid_file="$PWD/logs/nginx-itphlies.pid"
    nginx_config="$PWD/configs/nginx/itphlies-tunnel.conf"

    if [[ -f "$nginx_pid_file" ]]; then
        old_pid=$(cat "$nginx_pid_file")
        if kill -0 "$old_pid" >/dev/null 2>&1; then
            nginx -e "$PWD/logs/nginx-itphlies-error.log" -p "$PWD" -c "$nginx_config" -s quit || true
            sleep 1
            if kill -0 "$old_pid" >/dev/null 2>&1; then
                kill "$old_pid" || true
            fi
        fi
    fi

    nginx -e "$PWD/logs/nginx-itphlies-error.log" -p "$PWD" -c "$nginx_config"

    echo "ITPhlies nginx is up on http://localhost:8080 and http://itphlies:8080"
    echo "Nginx PID file: $nginx_pid_file"

stop-itphlies-deploy:
    #!/usr/bin/env bash
    set -euo pipefail

    backend_pid_file="$PWD/logs/itphlies-backend.pid"
    frontend_pid_file="$PWD/logs/itphlies-frontend.pid"
    nginx_pid_file="$PWD/logs/nginx-itphlies.pid"
    nginx_config="$PWD/configs/nginx/itphlies-tunnel.conf"

    mkdir -p logs

    just stop
    just stop-kill

    mapfile -t stale_release_worker_pids < <(pgrep -f "{{release_bin}} node run" || true)
    for pid in "${stale_release_worker_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
            echo "killed stale deployed worker pid=$pid"
        fi
    done

    mapfile -t stale_dev_worker_pids < <(pgrep -f "{{bin}} node run" || true)
    for pid in "${stale_dev_worker_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
            echo "killed stale dev worker pid=$pid"
        fi
    done

    if [[ -f "$frontend_pid_file" ]]; then
        frontend_pid=$(cat "$frontend_pid_file")
        if kill -0 "$frontend_pid" >/dev/null 2>&1; then
            kill "$frontend_pid" || true
            wait "$frontend_pid" >/dev/null 2>&1 || true
            echo "stopped itphlies frontend (pid=$frontend_pid)"
        fi
        rm -f "$frontend_pid_file"
    else
        echo "itphlies frontend already stopped"
    fi

    if [[ -f "$backend_pid_file" ]]; then
        backend_pid=$(cat "$backend_pid_file")
        if kill -0 "$backend_pid" >/dev/null 2>&1; then
            kill "$backend_pid" || true
            wait "$backend_pid" >/dev/null 2>&1 || true
            echo "stopped itphlies backend (pid=$backend_pid)"
        fi
        rm -f "$backend_pid_file"
    else
        echo "itphlies backend already stopped"
    fi

    mapfile -t stale_backend_pids < <(pgrep -f "{{release_bin}} server" || true)
    for pid in "${stale_backend_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
            echo "killed stale backend pid=$pid"
        fi
    done

    mapfile -t stale_dev_backend_pids < <(pgrep -f "{{bin}} server" || true)
    for pid in "${stale_dev_backend_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
            echo "killed stale dev backend pid=$pid"
        fi
    done

    if [[ -f "$nginx_pid_file" ]]; then
        nginx_pid="$(cat "$nginx_pid_file" 2>/dev/null || true)"
        nginx -e "$PWD/logs/nginx-itphlies-error.log" -p "$PWD" -c "$nginx_config" -s quit || true
        sleep 1
        if [[ -n "$nginx_pid" ]] && kill -0 "$nginx_pid" >/dev/null 2>&1; then
            kill "$nginx_pid" || true
        fi
        rm -f "$nginx_pid_file"
        echo "stopped itphlies nginx"
    else
        echo "itphlies nginx already stopped"
    fi

deploy-itphlies:
    #!/usr/bin/env bash
    set -euo pipefail

    frontend_pid_file="$PWD/logs/itphlies-frontend.pid"
    frontend_log_file="$PWD/logs/itphlies-frontend.log"

    just stop-itphlies-deploy
    just build-frontend
    just build-backend-release
    {{release_bin}} db start
    just deploy-itphlies-server
    just deploy-itphlies-nginx

    cd dashboard
    npx serve build >"$frontend_log_file" 2>&1 &
    frontend_pid=$!
    cd ..
    echo "$frontend_pid" > "$frontend_pid_file"

    echo "ITPhlies full deploy is up"
    echo "Frontend PID: $frontend_pid"
    echo "Frontend log: $frontend_log_file"
    echo "Open via tunnel: http://localhost:8080"
    echo "Open on LAN: http://itphlies:8080"

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
