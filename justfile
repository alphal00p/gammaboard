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

    stamp_file=".build-input.hash"
    current_hash="$(
        {
            printf '%s\n' package.json
            printf '%s\n' package-lock.json
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

serve-backend:
    {{bin}} server

serve-frontend:
    cd dashboard && npx serve build

deploy host mode="dev":
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "{{host}}" == "local" ]]; then
        just --justfile ops/local/justfile deploy "{{mode}}"
    elif [[ "{{host}}" == "itphlies" ]]; then
        just --justfile ops/itphlies/justfile deploy "{{mode}}"
    else
        echo "unknown deploy host: {{host}} (expected: local or itphlies)" >&2
        exit 1
    fi

deploy-status host="local":
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "{{host}}" == "local" ]]; then
        just --justfile ops/local/justfile status
    elif [[ "{{host}}" == "itphlies" ]]; then
        just --justfile ops/itphlies/justfile status
    else
        echo "unknown deploy host: {{host}} (expected: local or itphlies)" >&2
        exit 1
    fi

stop-deploy host="local":
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "{{host}}" == "local" ]]; then
        just --justfile ops/local/justfile down
    elif [[ "{{host}}" == "itphlies" ]]; then
        just --justfile ops/itphlies/justfile down
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
    #!/usr/bin/env bash
    set -euo pipefail

    scratch_root="${SCRATCH:-/scratch/network/users/${USER}}/apptainer"

    APPTAINER_TMPDIR="${APPTAINER_TMPDIR:-${scratch_root}/tmp}"
    APPTAINER_CACHEDIR="${APPTAINER_CACHEDIR:-${scratch_root}/cache}"
    SCCACHE_DIR="${SCCACHE_DIR:-/scratch/network/users/${USER}/gammaboard-cache/sccache}"
    SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-50G}"
    SCCACHE_NO_DAEMON="${SCCACHE_NO_DAEMON:-1}"

    mkdir -p "${APPTAINER_TMPDIR}" "${APPTAINER_CACHEDIR}" "${SCCACHE_DIR}"

    APPTAINER_TMPDIR="${APPTAINER_TMPDIR}" \
    APPTAINER_CACHEDIR="${APPTAINER_CACHEDIR}" \
    APPTAINERENV_SCCACHE_DIR="${SCCACHE_DIR}" \
    APPTAINERENV_SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE}" \
    APPTAINERENV_SCCACHE_NO_DAEMON="${SCCACHE_NO_DAEMON}" \
      apptainer build --notest gammaboard.sif ops/ubelix/build/gammaboard.def

sync-ubelix-ops host="Ubelix" remote_root="/storage/research/itp_localunitaritydata":
    just --justfile ops/ubelix/justfile sync-ops "{{host}}" "{{remote_root}}"

ubelix-tunnel compute_node host="Ubelix" local_port="8080" remote_port="8080":
    just --justfile ops/ubelix/justfile tunnel "{{compute_node}}" "{{host}}" "{{local_port}}" "{{remote_port}}"
