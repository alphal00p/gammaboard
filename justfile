test-e2e:
    cargo test -q --test full_stack_cli -- --ignored --nocapture --test-threads=1
