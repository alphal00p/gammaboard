worker_prefix := "worker"
poll_ms := "500"
run1_params_file := "configs/live-test-baseline-sin-scalar.toml"
run2_params_file := "configs/live-test-havana-sinc-complex.toml"
run3_params_file := "configs/live-test-invalid-sinc-scalar.toml"
run4_params_file := "configs/live-test-invalid-sin-complex.toml"

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
      NODE_ID="{{ worker_prefix }}-${i}"
      target/debug/run_node --node-id "${NODE_ID}" --poll-ms {{ poll_ms }} &
      echo "started ${NODE_ID}"
    done
    echo "{{ n }} workers started"

stop-workers:
    -pkill -f "cargo run --bin run_node -- .*--node-id"
    -pkill -f "{{ justfile_directory() }}/target/debug/run_node.*--node-id"
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
    just start-workers 12
    sleep 2

    create_run() {
      local cfg="$1"
      local out
      local id
      out=$(control_plane run-add --status pending --integration-params-file "${cfg}")
      id=$(echo "${out}" | sed -n 's/.*run_id=\([0-9]\+\).*/\1/p')
      if [[ -z "${id}" ]]; then
        echo "failed to parse run id from: ${out}" >&2
        exit 1
      fi
      echo "${id}"
    }

    RUN1_ID=$(create_run "{{ run1_params_file }}")
    RUN2_ID=$(create_run "{{ run2_params_file }}")
    RUN3_ID=$(create_run "{{ run3_params_file }}")
    RUN4_ID=$(create_run "{{ run4_params_file }}")

    control_plane assign --node-id "{{worker_prefix}}-1" --role evaluator --run-id "${RUN1_ID}"
    control_plane assign --node-id "{{worker_prefix}}-2" --role sampler-aggregator --run-id "${RUN1_ID}"

    for i in $(seq 3 7); do
      control_plane assign --node-id "{{worker_prefix}}-${i}" --role evaluator --run-id "${RUN2_ID}"
    done
    control_plane assign --node-id "{{worker_prefix}}-8" --role sampler-aggregator --run-id "${RUN2_ID}"

    control_plane assign --node-id "{{worker_prefix}}-9" --role evaluator --run-id "${RUN3_ID}"
    control_plane assign --node-id "{{worker_prefix}}-10" --role sampler-aggregator --run-id "${RUN3_ID}"

    control_plane assign --node-id "{{worker_prefix}}-11" --role evaluator --run-id "${RUN4_ID}"
    control_plane assign --node-id "{{worker_prefix}}-12" --role sampler-aggregator --run-id "${RUN4_ID}"

    control_plane run-start --run-id "${RUN1_ID}"
    control_plane run-start --run-id "${RUN2_ID}"
    control_plane run-start --run-id "${RUN3_ID}"
    control_plane run-start --run-id "${RUN4_ID}"

    echo "live-test started"
    echo "run1=${RUN1_ID} (baseline-sin-scalar): test_only_training + 1x test_only_sin + scalar (workers 1-2)"
    echo "run2=${RUN2_ID} (havana-sinc-complex): havana (2d) + 5x test_only_sinc + complex (workers 3-8)"
    echo "run3=${RUN3_ID} (invalid-sinc-with-scalar): test_only_training + 1x test_only_sinc + scalar (expected incompatibility error) (workers 9-10)"
    echo "run4=${RUN4_ID} (invalid-sin-with-complex): test_only_training + 1x test_only_sin + complex (expected incompatibility error) (workers 11-12)"

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
    @just stop-workers
    @just stop-serving
