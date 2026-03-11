set dotenv-load := true

poll_ms := "500"
#bin_debug := "./target/debug/gammaboard"
#bin_release := "./target/release/gammaboard"
#bin_profile := env_var_or_default("GAMMABOARD_BIN_PROFILE", "debug")

bin := "./target/dev-optim/gammaboard"

build:
    cargo build --profile dev-optim

check:
    cargo check --no-default-features --features 'cli,no_pyo3'

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
    {{bin}} server

build-frontend:
    cd dashboard && npm run build

serve-frontend:
    cd dashboard && npx serve build

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    just restart-db
    {{bin}} run-node --node-id "w-1" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-2" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-3" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-4" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-5" --poll-ms {{ poll_ms }} &

    sleep 4

    {{bin}} run add "configs/live-test-unit-naive-scalar.toml"
    {{bin}} run add "configs/symbolica-live-test.toml"

    {{bin}} node assign "w-1" evaluator 1
    {{bin}} node assign "w-2" sampler-aggregator 1

    {{bin}} node assign "w-3" evaluator 2
    {{bin}} node assign "w-4" evaluator 2
    {{bin}} node assign "w-5" sampler-aggregator 2

    {{bin}} run start 1 2

live-test-gammaloop:
    #!/usr/bin/env bash
    set -euo pipefail

    just restart-db
    {{bin}} run-node --node-id "w-0" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-1" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-2" --poll-ms {{ poll_ms }} &

    sleep 1

    {{bin}} run add "configs/gammaloop-triangle.toml"

    {{bin}} node assign "w-0" sampler-aggregator 1
    {{bin}} node assign "w-1" evaluator 1
    {{bin}} node assign "w-2" evaluator 1

    {{bin}} run start 1


stop:
    -{{bin}} run stop -a
    -{{bin}} node stop -a
    -@stty sane
