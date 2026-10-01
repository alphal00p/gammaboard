pub mod activity;
pub(crate) mod busy_time;
pub mod controller_child;
pub mod evaluator;
pub mod hyperparameter_tuning;
pub mod integration_campaign;
pub mod node_runner;
pub mod parameter_grid;
pub mod parameter_scan;
pub(crate) mod process_memory;
pub mod queue;
pub(crate) mod rolling_metric;
pub mod sampler_aggregator;
mod sampler_io;
pub(crate) mod stage_context;
pub mod task_control;
pub(crate) mod wall_time_rate;
pub(crate) mod window_metric;

pub use evaluator::{EvaluatorRunner, EvaluatorRunnerError, EvaluatorRunnerParams};
pub use node_runner::{NodeRunner, NodeRunnerConfig, NodeRunnerStore};
pub use queue::{QueueTickResult, SamplerQueue, SamplerQueueConfig};
pub use sampler_aggregator::{RunnerError, SamplerAggregatorRunner, SamplerAggregatorRunnerParams};
pub use task_control::{TaskControlLoop, TaskControlLoopConfig};

pub(crate) const MAX_EVALUATOR_DB_CONNECTIONS: u32 = 2;
/// Four inserts can overlap completion fetching and checkpoint/maintenance I/O.
pub(crate) const MAX_SAMPLER_DB_CONNECTIONS: u32 = 6;

pub(crate) fn role_db_pool_size(role: crate::core::WorkerRole, requested: u32) -> u32 {
    requested.clamp(
        1,
        match role {
            crate::core::WorkerRole::Evaluator => MAX_EVALUATOR_DB_CONNECTIONS,
            crate::core::WorkerRole::SamplerAggregator => MAX_SAMPLER_DB_CONNECTIONS,
        },
    )
}

#[cfg(test)]
pub(crate) mod test_support;
