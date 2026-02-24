worker_prefix := "worker"
poll_ms := "500"
run_params_file := "configs/live-test.toml"

install:
    cargo install --path . --bin control_plane

stop-db:
    docker-compose down -v

start-db:
    docker-compose up -d
    sqlx migrate run

restart-db:
    @just stop-db
    @just start-db

control-plane +args:
    cargo run --bin control_plane -- {{args}}

run-node node_id poll_ms='500':
    cargo run --bin run_node -- --node-id {{node_id}} --poll-ms {{poll_ms}}

start-workers n='10':
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --bin run_node
    for i in $(seq 1 {{n}}); do
      NODE_ID="{{worker_prefix}}-${i}"
      target/debug/run_node --node-id "${NODE_ID}" --poll-ms {{poll_ms}} &
      echo "started ${NODE_ID}"
    done
    echo "{{n}} workers started"

stop-workers:
    -pkill -f "cargo run --bin run_node -- .*--node-id"
    -pkill -f "{{justfile_directory()}}/target/debug/run_node.*--node-id"
    -pkill -f "target/debug/run_node --node-id"
    @echo "run_node workers stopped"

serve-backend:
    @echo "Starting Rust API server..."
    cargo run --bin server

serve-frontend:
    @echo "Starting frontend..."
    cd dashboard && npm start

serve:
    #!/usr/bin/env bash
    set -euo pipefail
    just serve-backend &
    just serve-frontend &
    echo "backend + frontend started in background"

live-test:
    #!/usr/bin/env bash
    set -euo pipefail

    just stop-workers || true
    just restart-db
    just start-workers 10
    sleep 2

    RUN1_OUT=$(control_plane run-add --status pending --integration-params-file {{run_params_file}})
    RUN1_ID=$(echo "${RUN1_OUT}" | sed -n 's/.*run_id=\([0-9]\+\).*/\1/p')
    if [[ -z "${RUN1_ID}" ]]; then
      echo "failed to parse run1 id from: ${RUN1_OUT}" >&2
      exit 1
    fi

    RUN2_OUT=$(control_plane run-add --status pending --integration-params-file {{run_params_file}})
    RUN2_ID=$(echo "${RUN2_OUT}" | sed -n 's/.*run_id=\([0-9]\+\).*/\1/p')
    if [[ -z "${RUN2_ID}" ]]; then
      echo "failed to parse run2 id from: ${RUN2_OUT}" >&2
      exit 1
    fi

    for i in $(seq 1 7); do
      control_plane assign --node-id "{{worker_prefix}}-${i}" --role evaluator --run-id "${RUN1_ID}"
    done
    control_plane assign --node-id "{{worker_prefix}}-8" --role sampler-aggregator --run-id "${RUN1_ID}"

    control_plane assign --node-id "{{worker_prefix}}-9" --role evaluator --run-id "${RUN2_ID}"
    control_plane assign --node-id "{{worker_prefix}}-10" --role sampler-aggregator --run-id "${RUN2_ID}"

    control_plane run-start --run-id "${RUN1_ID}"
    control_plane run-start --run-id "${RUN2_ID}"

    echo "live-test started"
    echo "run1=${RUN1_ID}: 7 evaluator + 1 sampler-aggregator (workers 1-8)"
    echo "run2=${RUN2_ID}: 1 evaluator + 1 sampler-aggregator (workers 9-10)"

stop-backend:
    -pkill -f "{{justfile_directory()}}/target/debug/server"
    -pkill -f "target/debug/server"
    -pkill -f "cargo run --bin server"
    @echo "Backend stopped"

stop-frontend:
    -pkill -f "gammaboard/dashboard.*react-scripts"
    @echo "Frontend stopped"

stop-serving:
    @just stop-backend
    @just stop-frontend

stop:
    @just stop-workers
    @just stop-serving
