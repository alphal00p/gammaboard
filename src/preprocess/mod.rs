mod preflight;

use crate::core::{
    BatchTransformConfig, BuildError, EvaluatorConfig, IntegrationParams, RunStageSnapshot,
    RunTaskInput, SamplerAggregatorConfig,
};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::NaiveMonteCarloSamplerParams;
use crate::utils::domain::Domain;
use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddIntegrationParams {
    pub evaluator: EvaluatorConfig,
    #[serde(default)]
    pub sampler_aggregator: Option<SamplerAggregatorConfig>,
    #[serde(default)]
    pub batch_transforms: Option<Vec<BatchTransformConfig>>,
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
    let resolved_batch_transforms = config
        .integration_params
        .batch_transforms
        .clone()
        .unwrap_or_default();
    let resolved_sampler_aggregator = config
        .integration_params
        .sampler_aggregator
        .clone()
        .unwrap_or_else(|| SamplerAggregatorConfig::NaiveMonteCarlo {
            params: NaiveMonteCarloSamplerParams::default(),
        });
    let resolved_task_queue = config.task_queue.clone();
    if let Some(tasks) = resolved_task_queue.as_ref() {
        for task in tasks {
            task.validate().map_err(BuildError::invalid_input)?;
        }
    }
    let resolved_integration_params = IntegrationParams {
        evaluator: config.integration_params.evaluator.clone(),
        sampler_aggregator: resolved_sampler_aggregator.clone(),
        batch_transforms: resolved_batch_transforms.clone(),
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

    // Determine an initial sample budget from the first resolved task when available.
    // This is used to construct an initial sampler for samplers that require a training
    // budget (e.g. HavanaTraining). Keep behavior minimal and in-place.
    let initial_sample_budget = resolved_task_queue.as_ref().and_then(|tasks| {
        tasks.first().and_then(|first_task| {
            // `nr_expected_samples` returns Option<i64>; convert to usize when possible.
            first_task
                .task
                .nr_expected_samples()
                .and_then(|n| usize::try_from(n).ok())
        })
    });

    let initial_stage_snapshot = preflight::build_initial_stage_with_budget(
        &resolved_sampler_aggregator,
        &resolved_batch_transforms,
        &domain,
        initial_sample_budget,
    )?;
    config.resolved_integration_params = Some(resolved_integration_params);
    config.initial_stage_snapshot = Some(initial_stage_snapshot);
    config.resolved_task_queue = resolved_task_queue;

    Ok(config)
}
