poll_ms := "500"

install:
    cargo install --path .
install-dev:
    cargo install --path . --debug

stop-db:
    docker-compose down -v

start-db:
    docker-compose up -d --wait
    sqlx migrate run

restart-db:
    @just stop-db
    @just start-db

serve-backend:
    @echo "Starting Rust API server..."
    @just stop-backend
    gammaboard server

serve-frontend:
    @echo "Starting frontend..."
    @just stop-frontend
    bash -lc 'set -a; [ -f .env ] && source .env; set +a; port="${GAMMABOOARD_BACKEND_PORT:?missing GAMMABOOARD_BACKEND_PORT}"; export REACT_APP_API_BASE_URL="http://localhost:$port/api"; cd dashboard && npm start'

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    just restart-db
    gammaboard run-node --node-id "w-1" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-2" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-3" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-4" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-5" --poll-ms {{ poll_ms }} &

    sleep 4

    gammaboard run add "configs/live-test-unit-naive-scalar.toml"
    gammaboard run add "configs/symbolica-live-test.toml"

    gammaboard node assign "w-1" evaluator 1
    gammaboard node assign "w-2" sampler-aggregator 1

    gammaboard node assign "w-3" evaluator 2
    gammaboard node assign "w-4" evaluator 2
    gammaboard node assign "w-5" sampler-aggregator 2

    gammaboard run start 1 2

live-test-gammaloop:
    #!/usr/bin/env bash
    set -euo pipefail

    just restart-db
    gammaboard run-node --node-id "w-1" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-2" --poll-ms {{ poll_ms }} &

    sleep 4

    gammaboard run add "configs/gammaloop-triangle.toml"

    gammaboard node assign "w-1" evaluator 1
    gammaboard node assign "w-2" sampler-aggregator 1

    gammaboard run start 1

stop-backend:
    -pkill -f "gammaboard server"
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
    -gammaboard run stop -a
    -gammaboard node stop -a
    -@just stop-serving
    -@stty sane
