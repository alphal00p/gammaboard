bin := "./target/dev-optim/gammaboard"
release_bin := "./target/release/gammaboard"

build-backend:
    cargo build --profile dev-optim

build-backend-release:
    cargo build --release

build:
    just build-frontend
    just build-backend

build-frontend:
    #!/usr/bin/env bash
    set -euo pipefail

    cd dashboard
    if [[ ! -x node_modules/.bin/react-scripts ]]; then
        npm ci
    fi
    npm run build

serve-backend:
    {{bin}} server

serve-frontend:
    cd dashboard && npx serve build

stop-deploy:
    {{bin}} deploy down

deploy-status:
    {{bin}} deploy status

deploy host mode="dev":
    {{bin}} deploy up --deploy-config "configs/deploy/{{host}}.toml" --mode "{{mode}}"

deploy-itphlies-dev:
    {{bin}} deploy up --deploy-config "configs/deploy/itphlies.toml" --mode "dev"

deploy-itphlies-release:
    {{bin}} deploy up --deploy-config "configs/deploy/itphlies.toml" --mode "release"

test-e2e:
    just build-backend
    just stop-kill
    cargo test -q --test full_stack_cli -- --ignored --nocapture

live-test-basic:
    #!/usr/bin/env bash
    set -euo pipefail

    run_live_test="live-test"
    run_symbolica_poly="symbolica-poly-test"
    run_symbolica_sin="symbolica-sin-test"

    {{bin}} db delete --yes
    {{bin}} db start
    {{bin}} node auto-run 8

    sleep 4

    {{bin}} run add "configs/runs/live-test-unit-naive-scalar.toml"
    {{bin}} run add "configs/runs/symbolica-live-test.toml"
    {{bin}} run add "configs/runs/symbolica-live-test-sin.toml"

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
    -timeout 5s {{bin}} run pause -a
    -timeout 5s {{bin}} node stop -a
    -@stty sane

stop-kill:
    just stop
    -pkill -f "{{bin}} node run"
    -pkill -f "{{bin}} server"
    -@stty sane

db-reset pg_stat_statements="false":
    #!/usr/bin/env bash
    set -euo pipefail

    just stop-kill
    {{bin}} db stop
    {{bin}} db delete --yes
    if [[ "{{pg_stat_statements}}" == "true" ]]; then
        {{bin}} db start --pg-stat-statements
    else
        {{bin}} db start
    fi
