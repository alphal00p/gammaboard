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
    cargo run --bin run_node -- --node-id "w-6" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-7" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-8" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-9" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-10" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-11" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-12" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-13" --poll-ms {{ poll_ms }} &
    cargo run --bin run_node -- --node-id "w-14" --poll-ms {{ poll_ms }} &

    sleep 4

    control_plane run-add "configs/live-test-baseline-sin-scalar.toml"
    control_plane run-add "configs/live-test-havana-sinc-complex.toml"
    control_plane run-add "configs/live-test-invalid-sinc-scalar.toml"
    control_plane run-add "configs/live-test-invalid-sin-complex.toml"
    control_plane run-add "configs/live-test-symbolica-unit-square-polynomial-scalar.toml"

    control_plane assign "w-1" evaluator 1
    control_plane assign "w-2" sampler-aggregator 1

    control_plane assign "w-3" evaluator 2
    control_plane assign "w-4" evaluator 2
    control_plane assign "w-5" evaluator 2
    control_plane assign "w-6" evaluator 2
    control_plane assign "w-7" evaluator 2
    control_plane assign "w-8" sampler-aggregator 2

    control_plane assign "w-9" evaluator 3
    control_plane assign "w-10" sampler-aggregator 3

    control_plane assign "w-11" evaluator 4
    control_plane assign "w-12" sampler-aggregator 4

    control_plane assign "w-13" evaluator 5
    control_plane assign "w-14" sampler-aggregator 5

    control_plane run-start 1 2 3 4 5

    echo "live-test started"
    echo "run1=1 (baseline-unit-sphere-volume): test_only_training + 1x unit + spherical parametrization + scalar (workers 1-2)"
    echo "run2=2 (havana-sinc-complex): havana (2d) + 5x test_only_sinc + complex (workers 3-8)"
    echo "run3=3 (invalid-sinc-with-scalar): test_only_training + 1x test_only_sinc + scalar (expected incompatibility error) (workers 9-10)"
    echo "run4=4 (invalid-sin-with-complex): test_only_training + 1x test_only_sin + complex (expected incompatibility error) (workers 11-12)"
    echo "run5=5 (symbolica-unit-square-x2-plus-y4-scalar): havana (2d) + 1x symbolica(x^2 + y^4) + scalar (workers 13-14)"

stop-backend:
    -pkill -f "{{ justfile_directory() }}/target/debug/server"
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
    @just control-plane "run-stop -a"
    @just control-plane "node-stop -a"
    @just stop-serving
