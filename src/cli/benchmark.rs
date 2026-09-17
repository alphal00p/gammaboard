use super::{performance::parse_duration, shared::print_json};
use anyhow::Result;
use clap::{Args, Subcommand};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Args)]
pub struct BenchmarkArgs {
    #[command(subcommand)]
    pub command: BenchmarkCommand,
}
#[derive(Debug, Subcommand)]
pub enum BenchmarkCommand {
    /// Calibrate fixed CPU work once, then reuse the iterations across the sweep
    Calibrate {
        #[arg(long)]
        eval_us: u64,
    },
    /// Direct uniform-sampling/scalar-accumulation baseline using a production evaluator
    Evaluator {
        config: PathBuf,
        #[arg(long, default_value_t = 1)]
        workers: usize,
        #[arg(long, default_value_t = 256)]
        batch_size: usize,
        #[arg(long, value_parser=parse_duration, conflicts_with="samples")]
        duration: Option<Duration>,
        #[arg(long, value_parser=parse_duration)]
        warmup: Option<Duration>,
        #[arg(long)]
        samples: Option<usize>,
    },
}
pub fn run(args: BenchmarkArgs) -> Result<()> {
    match args.command {
        BenchmarkCommand::Calibrate { eval_us } => print_json(&gammaboard::benchmark::calibrate(
            Duration::from_micros(eval_us),
        )?),
        BenchmarkCommand::Evaluator {
            config,
            workers,
            batch_size,
            duration,
            warmup,
            samples,
        } => {
            let workload = toml::from_str(&std::fs::read_to_string(config)?)?;
            print_json(&gammaboard::benchmark::direct(
                workload,
                workers,
                batch_size,
                duration.unwrap_or(Duration::from_secs(5)),
                warmup.unwrap_or(Duration::ZERO),
                samples,
            )?);
        }
    }
    Ok(())
}
