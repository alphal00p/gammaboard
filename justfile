set dotenv-load := true

poll_ms := "500"
bin := "./target/dev-optim/gammaboard"

build:
    cargo build --profile dev-optim

symlink-build:
    ln -sf "$(pwd)/target/dev-optim/gammaboard" "${HOME}/.cargo/bin/gammaboard"

install:
    cargo install --path . --profile dev-optim --force

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

start-workers n:
    #!/usr/bin/env bash
    set -euo pipefail

    for i in $(seq 1 {{n}}); do
        {{bin}} run-node --node-id "w-${i}" --poll-ms {{ poll_ms }} &
    done

start n:
    @just start-workers {{n}}

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    just restart-db
    just start 8

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
    just start 2

    sleep 1

    {{bin}} run add "configs/gammaloop-triangle.toml"

    {{bin}} node assign "w-0" sampler-aggregator 1
    {{bin}} node assign "w-1" evaluator 1
    {{bin}} node assign "w-2" evaluator 1

stop:
    -{{bin}} run pause -a
    -{{bin}} node stop -a
    -@stty sane
