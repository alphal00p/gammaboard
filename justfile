bin := "./target/dev-optim/gammaboard"

test-e2e:
    cargo build --profile dev-optim
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
