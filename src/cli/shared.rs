use clap::{Args, ValueEnum};
use gammaboard::core::{RunStatus, WorkerRole};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RoleArg {
    Evaluator,
    SamplerAggregator,
}

impl From<RoleArg> for WorkerRole {
    fn from(value: RoleArg) -> Self {
        match value {
            RoleArg::Evaluator => WorkerRole::Evaluator,
            RoleArg::SamplerAggregator => WorkerRole::SamplerAggregator,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RunStatusArg {
    Pending,
    WarmUp,
    Running,
    Completed,
    Paused,
    Cancelled,
}

impl From<RunStatusArg> for RunStatus {
    fn from(value: RunStatusArg) -> Self {
        match value {
            RunStatusArg::Pending => RunStatus::Pending,
            RunStatusArg::WarmUp => RunStatus::WarmUp,
            RunStatusArg::Running => RunStatus::Running,
            RunStatusArg::Completed => RunStatus::Completed,
            RunStatusArg::Paused => RunStatus::Paused,
            RunStatusArg::Cancelled => RunStatus::Cancelled,
        }
    }
}

#[derive(Debug, Args)]
pub struct RunSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "run_ids")]
    pub all: bool,
    #[arg(value_name = "RUN_ID", required_unless_present = "all")]
    pub run_ids: Vec<i32>,
}

#[derive(Debug, Args)]
pub struct NodeSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "node_ids")]
    pub all: bool,
    #[arg(value_name = "NODE_ID", required_unless_present = "all")]
    pub node_ids: Vec<String>,
}
