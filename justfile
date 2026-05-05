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
    if [[ ! -x node_modules/.bin/vite ]]; then
        npm ci
    fi

    stamp_file=".build-input.hash"
    current_hash="$(
        {
            printf '%s\n' package.json
            printf '%s\n' package-lock.json
            printf '%s\n' .nvmrc
            printf '%s\n' vite.config.js
            printf '%s\n' index.html
            find src -type f 2>/dev/null | sort
            find public -type f 2>/dev/null | sort
        } | while IFS= read -r path; do
            if [[ -f "${path}" ]]; then
                sha256sum "${path}"
            fi
        done | sha256sum | awk '{print $1}'
    )"

    if [[ -d build && -f "${stamp_file}" ]]; then
        previous_hash="$(cat "${stamp_file}")"
        if [[ "${previous_hash}" == "${current_hash}" ]]; then
            echo "frontend build unchanged; skipping npm run build"
            exit 0
        fi
    fi

    npm run build
    printf '%s\n' "${current_hash}" > "${stamp_file}"

reset-frontend-assets:
    #!/usr/bin/env bash
    set -euo pipefail

    cd dashboard
    rm -rf build
    rm -f .build-input.hash

serve-backend:
    {{bin}} server

serve-frontend:
    cd dashboard && npx serve build

deploy host mode="dev" port_offset="0":
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "{{host}}" == "local" ]]; then
        just --justfile ops/local/justfile deploy "{{mode}}" "{{port_offset}}"
    elif [[ "{{host}}" == "itphlies" ]]; then
        just --justfile ops/itphlies/justfile deploy "{{mode}}" "{{port_offset}}"
    else
        echo "unknown deploy host: {{host}} (expected: local or itphlies)" >&2
        exit 1
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

    {{bin}} run add "templates/runs/gammaloop.toml"
    {{bin}} run task add "$run_gammaloop" "templates/tasks/train_sample.toml"
    {{bin}} run add "templates/runs/python-scalar-python-sampler-flake-demo.toml"
    {{bin}} run add "templates/runs/symbolica-havana-pdf-1d2d.toml"

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
