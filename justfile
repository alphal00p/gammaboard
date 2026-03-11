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
    {{bin}} run-node --node-id "w-6" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-7" --poll-ms {{ poll_ms }} &
    {{bin}} run-node --node-id "w-8" --poll-ms {{ poll_ms }} &

    sleep 4

    {{bin}} run add "configs/live-test-unit-naive-scalar.toml"
    {{bin}} run add "configs/symbolica-live-test.toml"
    {{bin}} run add "configs/symbolica-live-test-sin.toml"

    {{bin}} node assign "w-1" evaluator 1
    {{bin}} node assign "w-2" sampler-aggregator 1

    {{bin}} node assign "w-3" sampler-aggregator 2
    {{bin}} node assign "w-4" evaluator 2
    {{bin}} node assign "w-5" evaluator 2
    {{bin}} node assign "w-6" evaluator 2
    {{bin}} node assign "w-7" evaluator 2
    {{bin}} node assign "w-8" evaluator 2

    echo "initial assignments settled"
    sleep 10

    echo "move two workers from run 2 to run 3"
    {{bin}} node assign "w-3" sampler-aggregator 3
    {{bin}} node assign "w-8" evaluator 3

    sleep 10

    echo "pause run 3 and return workers to run 2"
    {{bin}} run pause 3
    sleep 6
    {{bin}} node assign "w-3" sampler-aggregator 2
    {{bin}} node assign "w-8" evaluator 2

    sleep 10

    echo "pause run 2 and resume run 3 with all symbolica workers"
    {{bin}} run pause 2
    sleep 6
    {{bin}} node assign "w-3" sampler-aggregator 3
    {{bin}} node assign "w-4" evaluator 3
    {{bin}} node assign "w-5" evaluator 3
    {{bin}} node assign "w-6" evaluator 3
    {{bin}} node assign "w-7" evaluator 3
    {{bin}} node assign "w-8" evaluator 3

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

stop:
    -{{bin}} run pause -a
    -{{bin}} node stop -a
    -@stty sane
