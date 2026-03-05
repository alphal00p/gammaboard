poll_ms := "500"

install:
    cargo install --path .

install-dev:
    cargo install --path . --debug

stop-db:
    docker-compose down

reset-db:
    docker-compose down -v

start-db:
    docker-compose up -d --wait
    sqlx migrate run

restart-db:
    @just reset-db
    @just start-db

serve-backend:
    gammaboard server

serve-frontend:
    cd dashboard && npm start

build-frontend:
    cd dashboard && npm run build

serve-frontend-release:
    cd dashboard && npx serve build

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
    gammaboard run-node --node-id "w-0" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-1" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-2" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-3" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-4" --poll-ms {{ poll_ms }} &
    gammaboard run-node --node-id "w-5" --poll-ms {{ poll_ms }} &

    sleep 1

    gammaboard run add "configs/gammaloop-triangle.toml"

    gammaboard node assign "w-0" sampler-aggregator 1
    gammaboard node assign "w-1" evaluator 1
    gammaboard node assign "w-2" evaluator 1
    gammaboard node assign "w-3" evaluator 1
    gammaboard node assign "w-4" evaluator 1
    gammaboard node assign "w-5" evaluator 1

    gammaboard run start 1

stop:
    -gammaboard run stop -a
    -gammaboard node stop -a
    -@stty sane
