set dotenv-load := true
bin := "./target/dev-optim/gammaboard"

build-backend:
    cargo build --profile dev-optim

build:
    just build-frontend
    just build-backend

build-frontend:
    cd dashboard && npm run build

serve-backend:
    {{bin}} server

serve-frontend:
    cd dashboard && npx serve build

test-e2e:
    just stop-kill
    cargo test -q --test full_stack_cli -- --ignored --nocapture

start n:
    #!/usr/bin/env bash
    set -euo pipefail

    for i in $(seq 1 {{n}}); do
        {{bin}} run-node --name "w-${i}" &
    done

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    run_live_test="live-test"
    run_symbolica_poly="symbolica-poly-test"
    run_symbolica_sin="symbolica-sin-test"

    {{bin}} db delete --yes
    {{bin}} db start
    just start 8

    sleep 4

    {{bin}} run add "configs/live-test-unit-naive-scalar.toml"
    {{bin}} run add "configs/symbolica-live-test.toml"
    {{bin}} run add "configs/symbolica-live-test-sin.toml"

    {{bin}} node assign "w-1" evaluator "$run_live_test"
    {{bin}} node assign "w-2" sampler-aggregator "$run_live_test"

    {{bin}} auto-assign "$run_symbolica_poly" 5

    echo "initial assignments settled"
    sleep 10

    echo "move two workers from run 2 to run 3"
    {{bin}} node assign "w-3" sampler-aggregator "$run_symbolica_sin"
    {{bin}} node assign "w-8" evaluator "$run_symbolica_sin"

    sleep 10

    echo "pause run 3 and return workers to run 2"
    {{bin}} run pause "$run_symbolica_sin"
    sleep 6
    {{bin}} auto-assign "$run_symbolica_poly" 1

    sleep 10

    echo "pause run 2 and resume run 3 with all symbolica workers"
    {{bin}} run pause "$run_symbolica_poly"
    sleep 6
    {{bin}} auto-assign "$run_symbolica_sin" 5

stop:
    -{{bin}} run pause -a
    -{{bin}} node stop -a
    -@stty sane

stop-kill:
    just stop
    -pkill -f "{{bin}} run-node"
    -pkill -f "{{bin}} server"
    -@stty sane
