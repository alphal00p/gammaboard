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
    #!/usr/bin/env bash
    set -euo pipefail

    just build-frontend
    if [[ "{{mode}}" == "release" ]]; then
        just build-backend-release
        {{release_bin}} deploy up --deploy-config "configs/deploy/{{host}}.toml" --mode "{{mode}}"
    else
        just build-backend
        {{bin}} deploy up --deploy-config "configs/deploy/{{host}}.toml" --mode "{{mode}}"
    fi

test-e2e:
    just build-backend
    just stop-kill
    cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1

cli_example:
    #!/usr/bin/env bash
    set -euo pipefail

    run_gammaloop="gammaloop_tth"
    run_python="python-scalar-python-sampler-flake-demo"
    run_symbolica="symbolica-havana-pdf-1d2d"

    {{bin}} db delete --yes
    {{bin}} db start
    {{bin}} node auto-run 8

    sleep 4

    {{bin}} run add "configs/runs/gammaloop.toml"
    {{bin}} run task add "$run_gammaloop" "configs/tasks/train_sample.toml"
    {{bin}} run add "configs/runs/python-scalar-python-sampler-flake-demo.toml"
    {{bin}} run add "configs/runs/symbolica-havana-pdf-1d2d.toml"

    {{bin}} node assign "w-1" sampler-aggregator "$run_gammaloop"
    {{bin}} node assign "w-2" evaluator "$run_gammaloop"
    {{bin}} node assign "w-3" evaluator "$run_gammaloop"
    {{bin}} node assign "w-4" evaluator "$run_gammaloop"

    {{bin}} node assign "w-5" sampler-aggregator "$run_python"
    {{bin}} node assign "w-6" evaluator "$run_python"

    {{bin}} node assign "w-7" sampler-aggregator "$run_symbolica"
    {{bin}} node assign "w-8" evaluator "$run_symbolica"

    {{bin}} run list

stop:
    -timeout 5s {{bin}} run pause -a
    -timeout 5s {{bin}} node stop -a
    -@stty sane

stop-kill:
    just stop
    -pkill -f "{{bin}} node run"
    -pkill -f "{{bin}} server"
    -@stty sane

db-reset:
    #!/usr/bin/env bash
    set -euo pipefail

    just stop-kill
    {{bin}} db stop
    {{bin}} db delete --yes
    {{bin}} db start

build-apptainer:
    mkdir -p /var/tmp/${USER}
    APPTAINER_TMPDIR=/var/tmp/${USER} APPTAINER_CACHEDIR=/var/tmp/${USER} \
        apptainer build --notest gammaboard.sif gammaboard.def
