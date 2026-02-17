stop-db:
    docker-compose down -v

start-db:
    docker-compose up -d
    sqlx migrate run

restart-db:
    @just stop-db
    @just start-db

seed-mock-run:
    @echo "Ensuring mock run with id=1 exists..."
    docker-compose exec -T postgres psql -U postgres -d gammaboard_db -c "INSERT INTO runs (id, status, integration_params) VALUES (1, 'running', '{}'::jsonb) ON CONFLICT (id) DO NOTHING;"
    docker-compose exec -T postgres psql -U postgres -d gammaboard_db -c "SELECT setval(pg_get_serial_sequence('runs', 'id'), (SELECT GREATEST(COALESCE(MAX(id), 1), 1) FROM runs));"

run-mock-worker:
    @echo "Starting mock worker..."
    cargo run --bin mock_worker

run-mock-sampler-aggregator:
    @echo "Starting mock sampler-aggregator..."
    cargo run --bin mock_sampler_aggregator

stop-mock:
    -pkill -f "{{justfile_directory()}}/target/debug/mock_worker"
    -pkill -f "{{justfile_directory()}}/target/debug/mock_sampler_aggregator"
    -pkill -f "target/debug/mock_worker"
    -pkill -f "target/debug/mock_sampler_aggregator"
    @echo "Mock binaries stopped"

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
    just start-db
    just seed-mock-run
    trap 'just stop-mock || true; just stop-serving || true' EXIT INT TERM
    just serve-backend &
    sleep 2
    just run-mock-sampler-aggregator &
    just run-mock-worker &
    echo "Live test running (backend + mock sampler + mock worker). Press Ctrl+C to stop."
    wait

live-test-with-frontend:
    #!/usr/bin/env bash
    set -euo pipefail
    just start-db
    just seed-mock-run
    trap 'just stop-mock || true; just stop-serving || true' EXIT INT TERM
    just serve-backend &
    sleep 2
    just run-mock-sampler-aggregator &
    just run-mock-worker &
    just serve-frontend &
    echo "Live test running (backend + frontend + mock sampler + mock worker). Press Ctrl+C to stop."
    wait
