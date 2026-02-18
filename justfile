node_evaluator := "node-evaluator"
node_sampler := "node-sampler"
run_params_file := "configs/live-test.toml"

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

worker node_id poll_ms='500':
    cargo run --bin worker -- -t --node-id {{node_id}} --poll-ms {{poll_ms}}

run-evaluator-worker:
    @echo "Starting worker on {{node_evaluator}}..."
    just worker {{node_evaluator}} 500

run-sampler-worker:
    @echo "Starting worker on {{node_sampler}}..."
    just worker {{node_sampler}} 500

stop-workers:
    -pkill -f "cargo run --bin worker -- .*--node-id {{node_evaluator}}"
    -pkill -f "cargo run --bin worker -- .*--node-id {{node_sampler}}"
    -pkill -f "{{justfile_directory()}}/target/debug/worker.*--node-id {{node_evaluator}}"
    -pkill -f "{{justfile_directory()}}/target/debug/worker.*--node-id {{node_sampler}}"
    -pkill -f "target/debug/worker.*--node-id {{node_evaluator}}"
    -pkill -f "target/debug/worker.*--node-id {{node_sampler}}"
    @echo "Control-plane workers stopped"

serve-backend:
    @echo "Starting Rust API server..."
    cargo run --bin server

serve-frontend:
    @echo "Starting frontend..."
    cd dashboard && npm start

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

live-test:
    #!/usr/bin/env bash
    set -euo pipefail
    just stop-workers || true
    just stop-serving || true
    just restart-db

    just serve-backend &
    sleep 2
    just run-sampler-worker &
    just run-evaluator-worker &
    sleep 2

    CREATE_OUT=$(just control-plane run-add --status pending --integration-params-file {{run_params_file}})
    echo "${CREATE_OUT}"
    RUN_ID=$(echo "${CREATE_OUT}" | sed -n 's/.*run_id=\([0-9]\+\).*/\1/p')
    if [[ -z "${RUN_ID}" ]]; then
      echo "failed to parse run_id from control_plane run-add output" >&2
      exit 1
    fi

    just control-plane assign --node-id {{node_evaluator}} --role evaluator --run-id "${RUN_ID}"
    just control-plane assign --node-id {{node_sampler}} --role sampler-aggregator --run-id "${RUN_ID}"
    just control-plane run-start --run-id "${RUN_ID}"

    echo "Live test running (run_id=${RUN_ID}, backend + control-plane workers)."
    echo "Use 'just stop-workers' and 'just stop-serving' to stop processes."
    wait

live-test-with-frontend:
    #!/usr/bin/env bash
    set -euo pipefail
    just stop-workers || true
    just stop-serving || true
    just restart-db

    just serve-backend &
    sleep 2
    just run-sampler-worker &
    just run-evaluator-worker &
    sleep 2

    CREATE_OUT=$(just control-plane run-add --status pending --integration-params-file {{run_params_file}})
    echo "${CREATE_OUT}"
    RUN_ID=$(echo "${CREATE_OUT}" | sed -n 's/.*run_id=\([0-9]\+\).*/\1/p')
    if [[ -z "${RUN_ID}" ]]; then
      echo "failed to parse run_id from control_plane run-add output" >&2
      exit 1
    fi

    just control-plane assign --node-id {{node_evaluator}} --role evaluator --run-id "${RUN_ID}"
    just control-plane assign --node-id {{node_sampler}} --role sampler-aggregator --run-id "${RUN_ID}"
    just control-plane run-start --run-id "${RUN_ID}"

    just serve-frontend &
    echo "Live test running (run_id=${RUN_ID}, backend + frontend + control-plane workers)."
    echo "Use 'just stop-workers' and 'just stop-serving' to stop processes."
    wait
