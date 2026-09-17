use super::shared::{json_output_enabled, print_json};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use clap::{Args, ValueEnum};
use gammaboard::{PgStore, api::performance};
use std::time::{Duration, Instant};

pub fn parse_duration(raw: &str) -> Result<Duration, String> {
    let (number, scale) = if let Some(v) = raw.strip_suffix("ms") {
        (v, 0.001)
    } else if let Some(v) = raw.strip_suffix('s') {
        (v, 1.0)
    } else if let Some(v) = raw.strip_suffix('m') {
        (v, 60.0)
    } else {
        (raw, 1.0)
    };
    let seconds = number
        .parse::<f64>()
        .map_err(|_| "expected a duration such as 250ms, 30s, or 2m")?
        * scale;
    if !seconds.is_finite() || !(0.001..=86400.0).contains(&seconds) {
        return Err("duration must be between 1ms and 24h".into());
    }
    Ok(Duration::from_secs_f64(seconds))
}

#[derive(Debug, Args)]
pub struct PerformanceArgs {
    pub run: String,
    #[arg(long, value_parser=parse_duration, conflicts_with="since")]
    pub duration: Option<Duration>,
    #[arg(long, default_value="1s", value_parser=parse_duration)]
    pub interval: Duration,
    #[arg(long, default_value="10s", value_parser=parse_duration)]
    pub max_age: Duration,
    #[arg(long)]
    pub since: Option<DateTime<Utc>>,
    #[arg(long, requires = "since")]
    pub until: Option<DateTime<Utc>>,
    #[arg(long, default_value_t = 1000)]
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WaitUntil {
    Ready,
    Idle,
    Completed,
}

#[derive(Debug, Args)]
pub struct WaitArgs {
    pub run: String,
    #[arg(long, value_enum, default_value = "ready")]
    pub until: WaitUntil,
    #[arg(long, default_value_t = 1)]
    pub evaluators: usize,
    #[arg(long, default_value="60s", value_parser=parse_duration)]
    pub timeout: Duration,
    #[arg(long, default_value="10s", value_parser=parse_duration)]
    pub max_age: Duration,
}

pub async fn inspect(store: &PgStore, run_id: i32, args: PerformanceArgs) -> Result<()> {
    let value = if let Some(since) = args.since {
        performance::history(
            store,
            run_id,
            since,
            args.until.unwrap_or_else(Utc::now),
            args.limit,
        )
        .await?
    } else if let Some(duration) = args.duration {
        ensure!(
            args.interval >= Duration::from_millis(100),
            "measurement interval must be at least 100ms"
        );
        serde_json::to_value(
            performance::measure(store, run_id, duration, args.interval, args.max_age).await?,
        )?
    } else {
        serde_json::to_value(performance::snapshot(store, run_id).await?)?
    };
    if json_output_enabled() {
        print_json(&value);
    } else if let Some(valid) = value["valid"].as_bool() {
        println!(
            "Measurement: {} ({:.3}s)",
            if valid { "valid" } else { "INVALID" },
            value["elapsed_seconds"].as_f64().unwrap_or(0.0)
        );
        println!("Completed samples: {}", value["completed_samples"]);
        if let Some(rate) = value["samples_per_second"].as_f64() {
            println!("Throughput: {rate:.2} samples/s");
        }
        println!(
            "Allocated worker core-seconds: {}",
            value["allocated_core_seconds"]
        );
        for issue in value["issues"].as_array().into_iter().flatten() {
            println!("  {}", issue.as_str().unwrap_or("unknown issue"));
        }
    } else if let Some(rows) = value["rows"].as_array() {
        println!(
            "{} historical rows; truncated={}. Use --json for raw data.",
            rows.len(),
            value["truncated"]
        );
    } else {
        let snapshot: performance::PerformanceSnapshot = serde_json::from_value(value)?;
        println!(
            "Run {} ({}) — task {}",
            snapshot.run_id,
            snapshot.run_name,
            snapshot.task_id.as_deref().unwrap_or("none")
        );
        println!("Completed samples: {}", snapshot.completed_samples);
        println!("Active evaluators: {}", snapshot.active_evaluators());
        println!(
            "Allocated worker core-seconds: {:.3}",
            snapshot.allocated_core_seconds
        );
        println!("Queue: {}", snapshot.queue);
        for issue in snapshot.coverage_issues(args.max_age) {
            println!("  {issue}");
        }
    }
    Ok(())
}

pub async fn wait(store: &PgStore, run_id: i32, args: WaitArgs) -> Result<()> {
    ensure!(args.evaluators > 0, "evaluators must be positive");
    let started = Instant::now();
    loop {
        let s = performance::snapshot(store, run_id).await?;
        if !matches!(args.until, WaitUntil::Idle) && s.failed_tasks > 0 {
            bail!("run {run_id} has failed tasks");
        }
        let done = match args.until {
            WaitUntil::Ready => s.ready(args.evaluators, args.max_age),
            WaitUntil::Idle => s.idle(),
            WaitUntil::Completed => s.unfinished_tasks == 0,
        };
        if done {
            if json_output_enabled() {
                print_json(&s);
            } else {
                println!("Run {run_id}: {:?}", args.until);
            }
            return Ok(());
        }
        ensure!(
            started.elapsed() < args.timeout,
            "timed out waiting for run {run_id}: {:?}; coverage: {:?}",
            args.until,
            s.coverage_issues(args.max_age)
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
