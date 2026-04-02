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

_deploy-backend host mode:
    #!/usr/bin/env bash
    set -euo pipefail

    host="{{host}}"
    mode="{{mode}}"

    case "$host" in
        local)
            server_config="configs/server/local-prod.toml"
            ;;
        itphlies)
            server_config="configs/server/itphlies-prod.toml"
            ;;
        *)
            echo "unsupported deploy host: $host" >&2
            exit 1
            ;;
    esac

    case "$mode" in
        dev)
            backend_bin="{{bin}}"
            ;;
        release)
            backend_bin="{{release_bin}}"
            ;;
        *)
            echo "unsupported deploy mode: $mode" >&2
            exit 1
            ;;
    esac

    backend_pid_file="$PWD/logs/deploy-backend.pid"
    backend_log_file="$PWD/logs/deploy-backend.log"

    mkdir -p logs

    if [[ -f "$backend_pid_file" ]]; then
        old_pid=$(cat "$backend_pid_file")
        if kill -0 "$old_pid" >/dev/null 2>&1; then
            kill "$old_pid" >/dev/null 2>&1 || true
            wait "$old_pid" >/dev/null 2>&1 || true
        fi
    fi

    "$backend_bin" server --server-config "$server_config" >"$backend_log_file" 2>&1 &
    new_pid=$!
    echo "$new_pid" > "$backend_pid_file"

    echo "Deployed backend on $(hostname)"
    echo "Backend PID: $new_pid"
    echo "Backend log: $backend_log_file"
    echo "Server config: $server_config"
    echo "Mode: $mode"

_deploy-nginx host:
    #!/usr/bin/env bash
    set -euo pipefail

    host="{{host}}"

    mkdir -p logs tmp/nginx/client_body tmp/nginx/proxy tmp/nginx/fastcgi tmp/nginx/uwsgi tmp/nginx/scgi

    case "$host" in
        local)
            nginx_config="$PWD/configs/nginx/local-prod.conf"
            open_message="Open: http://localhost:8080"
            ;;
        itphlies)
            nginx_config="$PWD/configs/nginx/itphlies-tunnel.conf"
            open_message="Open via tunnel: http://localhost:8080"$'\n'"Open on LAN: http://itphlies:8080"
            ;;
        *)
            echo "unsupported deploy host: $host" >&2
            exit 1
            ;;
    esac

    nginx_pid_file="$PWD/logs/nginx-deploy.pid"
    nginx_error_log="$PWD/logs/nginx-deploy-error.log"

    if [[ -f "$nginx_pid_file" ]]; then
        old_pid=$(cat "$nginx_pid_file")
        if kill -0 "$old_pid" >/dev/null 2>&1; then
            nginx -e "$nginx_error_log" -p "$PWD" -c "$nginx_config" -s quit || true
            sleep 1
            if kill -0 "$old_pid" >/dev/null 2>&1; then
                kill "$old_pid" || true
            fi
        fi
    fi

    nginx -e "$nginx_error_log" -p "$PWD" -c "$nginx_config"

    echo "Deploy nginx is up"
    echo "Nginx PID file: $nginx_pid_file"
    printf '%s\n' "$open_message"

_deploy-frontend host:
    #!/usr/bin/env bash
    set -euo pipefail

    host="{{host}}"

    case "$host" in
        local|itphlies)
            ;;
        *)
            echo "unsupported deploy host: $host" >&2
            exit 1
            ;;
    esac

    frontend_pid_file="$PWD/logs/deploy-frontend.pid"
    frontend_log_file="$PWD/logs/deploy-frontend.log"

    mkdir -p logs

    cd dashboard
    if [[ -f "$frontend_pid_file" ]]; then
        old_pid=$(cat "$frontend_pid_file")
        if kill -0 "$old_pid" >/dev/null 2>&1; then
            kill "$old_pid" >/dev/null 2>&1 || true
            wait "$old_pid" >/dev/null 2>&1 || true
        fi
    fi
    npx serve build >"$frontend_log_file" 2>&1 &
    frontend_pid=$!
    cd ..
    echo "$frontend_pid" > "$frontend_pid_file"

    echo "Deploy frontend is up"
    echo "Frontend PID: $frontend_pid"
    echo "Frontend log: $frontend_log_file"

stop-deploy:
    #!/usr/bin/env bash
    set -euo pipefail

    backend_pid_file="$PWD/logs/deploy-backend.pid"
    frontend_pid_file="$PWD/logs/deploy-frontend.pid"
    nginx_pid_file="$PWD/logs/nginx-deploy.pid"

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
            kill "$frontend_pid" >/dev/null 2>&1 || true
            wait "$frontend_pid" >/dev/null 2>&1 || true
            echo "stopped deploy frontend (pid=$frontend_pid)"
        fi
        rm -f "$frontend_pid_file"
    else
        echo "deploy frontend already stopped"
    fi

    if [[ -f "$backend_pid_file" ]]; then
        backend_pid=$(cat "$backend_pid_file")
        if kill -0 "$backend_pid" >/dev/null 2>&1; then
            kill "$backend_pid" >/dev/null 2>&1 || true
            wait "$backend_pid" >/dev/null 2>&1 || true
            echo "stopped deploy backend (pid=$backend_pid)"
        fi
        rm -f "$backend_pid_file"
    else
        echo "deploy backend already stopped"
    fi

    mapfile -t stale_backend_pids < <(pgrep -f "{{release_bin}} server" || true)
    for pid in "${stale_backend_pids[@]}"; do
        if [[ -n "$pid" ]]; then
            kill "$pid" >/dev/null 2>&1 || true
            wait "$pid" >/dev/null 2>&1 || true
            echo "killed stale release backend pid=$pid"
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
        nginx -e "$PWD/logs/nginx-deploy-error.log" -p "$PWD" -c "$PWD/configs/nginx/local-prod.conf" -s quit || true
        nginx -e "$PWD/logs/nginx-deploy-error.log" -p "$PWD" -c "$PWD/configs/nginx/itphlies-tunnel.conf" -s quit || true
        sleep 1
        if [[ -n "$nginx_pid" ]] && kill -0 "$nginx_pid" >/dev/null 2>&1; then
            kill "$nginx_pid" || true
        fi
        rm -f "$nginx_pid_file"
        echo "stopped deploy nginx"
    else
        echo "deploy nginx already stopped"
    fi

deploy host mode="dev":
    #!/usr/bin/env bash
    set -euo pipefail

    host="{{host}}"
    mode="{{mode}}"

    case "$mode" in
        dev)
            backend_bin="{{bin}}"
            build_recipe="build-backend"
            ;;
        release)
            backend_bin="{{release_bin}}"
            build_recipe="build-backend-release"
            ;;
        *)
            echo "unsupported deploy mode: $mode" >&2
            exit 1
            ;;
    esac

    case "$host" in
        local)
            open_message="Open: http://localhost:8080"
            ;;
        itphlies)
            open_message="Open via tunnel: http://localhost:8080"$'\n'"Open on LAN: http://itphlies:8080"
            ;;
        *)
            echo "unsupported deploy host: $host" >&2
            exit 1
            ;;
    esac

    just stop-deploy
    just build-frontend
    just "$build_recipe"
    "$backend_bin" db start
    just _deploy-backend "$host" "$mode"
    just _deploy-nginx "$host"
    just _deploy-frontend "$host"

    echo "Deploy is up"
    echo "Host: $host"
    echo "Mode: $mode"
    printf '%s\n' "$open_message"

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
    -timeout 5s {{bin}} run pause -a
    -timeout 5s {{bin}} node stop -a
    -@stty sane

stop-kill:
    just stop
    -pkill -f "{{bin}} node run"
    -pkill -f "{{bin}} server"
    -@stty sane

db-reset pg_stat_statements="false":
    #!/usr/bin/env bash
    set -euo pipefail

    just stop-kill
    {{bin}} db stop
    {{bin}} db delete --yes
    if [[ "{{pg_stat_statements}}" == "true" ]]; then
        {{bin}} db start --pg-stat-statements
    else
        {{bin}} db start
    fi
