#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/benchmark_campaign.sh [--duration-seconds N] [--workers N] [--port-offset N]

Runs a sustained, dependency-free local campaign and reports throughput, queue
depth, database connections, database growth, and performance-history growth.
It starts an isolated local deployment, removes the benchmark run, and stops
the deployment when finished.

Environment:
  GAMMABOARD_BENCHMARK_DATABASE_URL  Override the local PostgreSQL URL used for
                                      measurements.
EOF
}

duration_seconds=600
workers=4
port_offset=30

while (($#)); do
    case "$1" in
        --duration-seconds)
            duration_seconds="$2"
            shift 2
            ;;
        --workers)
            workers="$2"
            shift 2
            ;;
        --port-offset)
            port_offset="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if ! [[ "$duration_seconds" =~ ^[1-9][0-9]*$ ]] || ! [[ "$workers" =~ ^[2-9][0-9]*$ ]] || ! [[ "$port_offset" =~ ^[0-9]+$ ]]; then
    echo "duration must be positive, workers must be at least 2, and port offset must be non-negative" >&2
    exit 2
fi

for command in curl psql awk; do
    command -v "$command" >/dev/null || {
        echo "missing required command '$command'; run 'nix develop' first" >&2
        exit 127
    }
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

api_port=$((4000 + port_offset))
postgres_port=$((5400 + port_offset))
database_url="${GAMMABOARD_BENCHMARK_DATABASE_URL:-postgresql://postgres:NqVj2yt5WsCE5nYCOx01MkeFD8n8awoZ@127.0.0.1:${postgres_port}/gammaboard_db}"
run_name="campaign-benchmark-$(date +%s)-$$"
config_file="$(mktemp)"
deploy_log="$(mktemp)"
deploy_pid=""
started_nodes=false
created_run=false

psql_value() {
    psql "$database_url" --tuples-only --no-align --quiet -c "$1"
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    set +e
    if [[ "$started_nodes" == true ]]; then
        ./gammaboard --port-offset "$port_offset" node stop -a >/dev/null 2>&1
    fi
    if [[ "$created_run" == true ]]; then
        ./gammaboard --port-offset "$port_offset" run remove --yes "$run_name" >/dev/null 2>&1
    fi
    if [[ -n "$deploy_pid" ]]; then
        kill -INT "$deploy_pid" >/dev/null 2>&1
        wait "$deploy_pid" >/dev/null 2>&1
    fi
    rm -f "$config_file" "$deploy_log"
    exit "$status"
}
trap cleanup EXIT INT TERM

cat >"$config_file" <<EOF
name = "$run_name"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "campaign"
kind = "sample"
stop_condition = { max_samples = 1_000_000_000 }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 0 } }

[task_queue.queue_tuning]
max_batch_size = 10_000
target_batch_eval_ms = 200.0

[evaluator_runner_params]
min_tick_time_ms = 500
performance_snapshot_interval_ms = 2_000

[sampler_aggregator_runner_params]
min_tick_time_ms = 500
performance_snapshot_interval_ms = 2_000
EOF

echo "starting isolated deployment on port offset $port_offset"
./gammaboard --port-offset "$port_offset" deploy >"$deploy_log" 2>&1 &
deploy_pid=$!
for _ in $(seq 1 120); do
    if curl --fail --silent "http://127.0.0.1:${api_port}/api/health" >/dev/null; then
        break
    fi
    if ! kill -0 "$deploy_pid" 2>/dev/null; then
        cat "$deploy_log" >&2
        exit 1
    fi
    sleep 1
done
curl --fail --silent "http://127.0.0.1:${api_port}/api/health" >/dev/null || {
    cat "$deploy_log" >&2
    exit 1
}

./gammaboard --port-offset "$port_offset" run create "$config_file"
created_run=true
run_id="$(psql_value "SELECT id FROM runs WHERE name = '$run_name'")"

./gammaboard --port-offset "$port_offset" node start-local "$workers"
started_nodes=true
sleep 2
./gammaboard --port-offset "$port_offset" node auto-assign "$run_name"

start_epoch="$(date +%s)"
start_samples="$(psql_value "SELECT nr_completed_samples FROM runs WHERE id = $run_id")"
start_db_bytes="$(psql_value 'SELECT pg_database_size(current_database())')"
start_evaluator_rows="$(psql_value "SELECT count(*) FROM evaluator_performance_history WHERE run_id = $run_id")"
start_sampler_rows="$(psql_value "SELECT count(*) FROM sampler_aggregator_performance_history WHERE run_id = $run_id")"
max_pending=0
max_claimed=0
max_connections=0

echo "benchmarking $workers workers for ${duration_seconds}s"
while (( $(date +%s) - start_epoch < duration_seconds )); do
    queue_stats="$(psql_value "
        SELECT
            count(*) FILTER (WHERE status = 'pending'),
            count(*) FILTER (WHERE status = 'claimed'),
            (SELECT count(*) FROM pg_stat_activity WHERE datname = current_database())
        FROM batches
        WHERE run_id = $run_id
    " | tr -d '[:space:]')"
    IFS='|' read -r pending claimed connections <<<"$queue_stats"
    if (( pending > max_pending )); then
        max_pending=$pending
    fi
    if (( claimed > max_claimed )); then
        max_claimed=$claimed
    fi
    if (( connections > max_connections )); then
        max_connections=$connections
    fi
    sleep 5
done

end_epoch="$(date +%s)"
end_samples="$(psql_value "SELECT nr_completed_samples FROM runs WHERE id = $run_id")"
end_db_bytes="$(psql_value 'SELECT pg_database_size(current_database())')"
end_evaluator_rows="$(psql_value "SELECT count(*) FROM evaluator_performance_history WHERE run_id = $run_id")"
end_sampler_rows="$(psql_value "SELECT count(*) FROM sampler_aggregator_performance_history WHERE run_id = $run_id")"
error_logs="$(psql_value "SELECT count(*) FROM runtime_logs WHERE run_id = $run_id AND level = 'ERROR'")"
elapsed_seconds=$((end_epoch - start_epoch))
completed_samples=$((end_samples - start_samples))
telemetry_rows=$((end_evaluator_rows - start_evaluator_rows + end_sampler_rows - start_sampler_rows))
database_growth=$((end_db_bytes - start_db_bytes))
samples_per_second="$(awk -v samples="$completed_samples" -v seconds="$elapsed_seconds" 'BEGIN { printf "%.2f", samples / seconds }')"
telemetry_rows_per_day="$(awk -v rows="$telemetry_rows" -v seconds="$elapsed_seconds" 'BEGIN { printf "%.0f", rows * 86400 / seconds }')"

cat <<EOF

Campaign benchmark result
workers: $workers (1 sampler, $((workers - 1)) evaluators)
duration_seconds: $elapsed_seconds
completed_samples: $completed_samples
completed_samples_per_second: $samples_per_second
max_pending_batches: $max_pending
max_claimed_batches: $max_claimed
max_database_connections: $max_connections
performance_history_rows: $telemetry_rows
projected_performance_history_rows_per_day: $telemetry_rows_per_day
database_growth_bytes: $database_growth
error_runtime_logs: $error_logs
EOF
