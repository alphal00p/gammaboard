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
pub(crate) mod stage_context;
pub mod task_control;
pub(crate) mod wall_time_rate;
pub(crate) mod window_metric;

pub use evaluator::{EvaluatorRunner, EvaluatorRunnerError, EvaluatorRunnerParams};
pub use node_runner::{NodeRunner, NodeRunnerConfig, NodeRunnerStore};
pub use queue::{QueueTickResult, SamplerQueue, SamplerQueueConfig};
pub use sampler_aggregator::{RunnerError, SamplerAggregatorRunner, SamplerAggregatorRunnerParams};
pub use task_control::{TaskControlLoop, TaskControlLoopConfig};

/// Maximum role connections per worker; launch admission reserves two more for control.
pub const MAX_ROLE_DB_CONNECTIONS_PER_NODE: u32 = 2;
