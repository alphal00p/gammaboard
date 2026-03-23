mod preflight;

use crate::core::{
    BuildError, EvaluatorConfig, IntegrationParams, ParametrizationConfig, RunStageSnapshot,
    RunTaskInputSpec, RunTaskSpec, SamplerAggregatorConfig, resolve_initial_sampler_aggregator,
    resolve_task_queue,
};
use crate::evaluation::PointSpec;
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::NaiveMonteCarloSamplerParams;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub use preflight::preflight_task_suffix;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddIntegrationParams {
    pub evaluator: EvaluatorConfig,
    #[serde(default)]
    pub sampler_aggregator: Option<SamplerAggregatorConfig>,
    #[serde(default)]
    pub parametrization: Option<ParametrizationConfig>,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddConfig {
    pub name: String,
    pub task_queue: Option<Vec<RunTaskInputSpec>>,
    #[serde(flatten)]
    pub integration_params: RunAddIntegrationParams,
    pub target: Option<JsonValue>,
    #[serde(skip)]
    pub point_spec: Option<PointSpec>,
    #[serde(skip)]
    pub resolved_integration_params: Option<IntegrationParams>,
    #[serde(skip)]
    pub evaluator_init_metadata: Option<JsonValue>,
    #[serde(skip)]
    pub sampler_aggregator_init_metadata: Option<JsonValue>,
    #[serde(skip)]
    pub initial_stage_snapshot: Option<RunStageSnapshot>,
    #[serde(skip)]
    pub resolved_task_queue: Option<Vec<RunTaskSpec>>,
}

pub fn preprocess_run_add(mut config: RunAddConfig) -> Result<RunAddConfig, BuildError> {
    let resolved_parametrization = config
        .integration_params
        .parametrization
        .clone()
        .unwrap_or_else(ParametrizationConfig::identity_default);
    let resolved_sampler_aggregator = resolve_initial_sampler_aggregator(
        config.task_queue.as_deref(),
        config.integration_params.sampler_aggregator.as_ref(),
    )
    .unwrap_or_else(|| SamplerAggregatorConfig::NaiveMonteCarlo {
        params: NaiveMonteCarloSamplerParams::default(),
    });
    let resolved_task_queue = config
        .task_queue
        .as_ref()
        .map(|tasks| {
            resolve_task_queue(
                &resolved_sampler_aggregator,
                &resolved_parametrization,
                tasks,
            )
        })
        .transpose()
        .map_err(BuildError::build)?;
    if let Some(tasks) = resolved_task_queue.as_ref() {
        for task in tasks {
            task.validate().map_err(BuildError::invalid_input)?;
        }
    }
    let resolved_integration_params = IntegrationParams {
        evaluator: config.integration_params.evaluator.clone(),
        sampler_aggregator: resolved_sampler_aggregator.clone(),
        parametrization: resolved_parametrization.clone(),
        evaluator_runner_params: config.integration_params.evaluator_runner_params.clone(),
        sampler_aggregator_runner_params: config
            .integration_params
            .sampler_aggregator_runner_params
            .clone(),
    };

    let evaluator = config.integration_params.evaluator.build()?;
    let point_spec = evaluator.get_point_spec();
    let evaluator_init_metadata = evaluator.get_init_metadata();
    config.point_spec = Some(point_spec.clone());

    let (sampler_aggregator_init_metadata, initial_stage_snapshot) =
        preflight::build_initial_stage(
            &resolved_sampler_aggregator,
            &resolved_parametrization,
            &point_spec,
        )?;
    config.resolved_integration_params = Some(resolved_integration_params);
    config.evaluator_init_metadata = Some(evaluator_init_metadata);
    config.sampler_aggregator_init_metadata = Some(sampler_aggregator_init_metadata);
    config.initial_stage_snapshot = Some(initial_stage_snapshot);
    config.resolved_task_queue = resolved_task_queue;

    Ok(config)
}
