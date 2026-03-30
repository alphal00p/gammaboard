mod preflight;

use crate::core::{BuildError, EvaluatorConfig, IntegrationParams, RunStageSnapshot, RunTaskInput};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::utils::domain::Domain;
use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddIntegrationParams {
    pub evaluator: EvaluatorConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddConfig {
    pub name: String,
    pub task_queue: Option<Vec<RunTaskInput>>,
    #[serde(flatten)]
    pub integration_params: RunAddIntegrationParams,
    pub target: Option<JsonValue>,
    #[serde(skip)]
    pub domain: Option<Domain>,
    #[serde(skip)]
    pub resolved_integration_params: Option<IntegrationParams>,
    #[serde(skip)]
    pub initial_stage_snapshot: Option<RunStageSnapshot>,
    #[serde(skip)]
    pub resolved_task_queue: Option<Vec<RunTaskInput>>,
}

pub fn preprocess_run_add(mut config: RunAddConfig) -> Result<RunAddConfig, BuildError> {
    let resolved_task_queue = config.task_queue.clone();
    if let Some(tasks) = resolved_task_queue.as_ref() {
        for task in tasks {
            task.validate().map_err(BuildError::invalid_input)?;
        }
    }
    let resolved_integration_params = IntegrationParams {
        evaluator: config.integration_params.evaluator.clone(),
        evaluator_runner_params: config.integration_params.evaluator_runner_params.clone(),
        sampler_aggregator_runner_params: config
            .integration_params
            .sampler_aggregator_runner_params
            .clone(),
    };

    let evaluator_kind = config.integration_params.evaluator.kind_str();
    let evaluator = config.integration_params.evaluator.build().map_err(|err| {
        BuildError::build(format!(
            "failed to initialize evaluator {evaluator_kind}: {err}"
        ))
    })?;
    let domain = evaluator.get_domain();
    config.domain = Some(domain.clone());

    let initial_stage_snapshot = preflight::build_initial_stage()?;
    config.resolved_integration_params = Some(resolved_integration_params);
    config.initial_stage_snapshot = Some(initial_stage_snapshot);
    config.resolved_task_queue = resolved_task_queue;

    Ok(config)
}
