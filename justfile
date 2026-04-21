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
