mod preflight;

use crate::core::{BuildError, EvaluatorConfig, IntegrationParams, RunStageSnapshot, RunTaskInput};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::utils::domain::Domain;
use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddIntegrationParams {
    #[serde(default)]
    pub evaluator: Option<EvaluatorConfig>,
    #[serde(default)]
    pub evaluator_requirements: crate::core::CapabilityRequirements,
    #[serde(default)]
    pub sampler_requirements: crate::core::CapabilityRequirements,
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
    pub original_toml: Option<String>,
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
        evaluator_requirements: config.integration_params.evaluator_requirements.clone(),
        sampler_requirements: config.integration_params.sampler_requirements.clone(),
        evaluator_runner_params: config.integration_params.evaluator_runner_params.clone(),
        sampler_aggregator_runner_params: config
            .integration_params
            .sampler_aggregator_runner_params
            .clone(),
    };

    let domain = match config.integration_params.evaluator.as_ref() {
        Some(evaluator) => {
            let evaluator_kind = evaluator.kind_str();
            evaluator.resolve_domain().map_err(|err| {
                BuildError::build(format!(
                    "failed to resolve evaluator domain for {evaluator_kind}: {err}"
                ))
            })?
        }
        None => Domain::continuous(0),
    };
    config.domain = Some(domain.clone());

    let initial_stage_snapshot = preflight::build_initial_stage()?;
    config.resolved_integration_params = Some(resolved_integration_params);
    config.initial_stage_snapshot = Some(initial_stage_snapshot);
    config.resolved_task_queue = resolved_task_queue;

    Ok(config)
}
