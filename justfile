poll_ms := "500"

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
    cargo run --bin control_plane -- {{ args }}

run-node node_id poll_ms='500':
    cargo run --bin run_node -- --node-id {{ node_id }} --poll-ms {{ poll_ms }}

start-workers n='10':
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --bin run_node
    for i in $(seq 1 {{ n }}); do
      NODE_ID="w-${i}"
      target/debug/run_node --node-id "${NODE_ID}" --poll-ms {{ poll_ms }} &
      echo "started ${NODE_ID}"
    done
    echo "{{ n }} workers started"

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

    just restart-db
    cargo run --bin run_node -- --node-id "w-1" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-2" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-3" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-4" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-5" --poll-ms {{ poll_ms }} &

    sleep 4

    control_plane run-add "configs/live-test-unit-naive-scalar.toml"
    control_plane run-add "configs/symbolica-live-test.toml"

    control_plane assign "w-1" evaluator 1
    control_plane assign "w-2" sampler-aggregator 1

    control_plane assign "w-3" evaluator 2
    control_plane assign "w-4" evaluator 2
    control_plane assign "w-5" sampler-aggregator 2

    control_plane run-start 1 2

stop-backend:
    -pkill -f "{{ justfile_directory() }}/target/debug/server"
    -pkill -f "target/debug/server"
    -pkill -f "cargo run --bin server"
    @echo "Backend stopped"

stop-frontend:
    #!/usr/bin/env bash
    set -euo pipefail
    # Prefer graceful stop to avoid noisy "terminated by signal 15" output.
    pkill -INT -f "gammaboard/dashboard.*react-scripts" || true
    sleep 0.5
    pkill -TERM -f "gammaboard/dashboard.*react-scripts" || true
    echo "Frontend stopped"

stop-serving:
    -@just stop-backend
    -@just stop-frontend

stop:
    -@just control-plane "run-stop -a"
    -@just control-plane "node-stop -a"
    -@just stop-serving
    -@stty sane
