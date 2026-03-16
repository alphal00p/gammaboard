use crate::core::{
    BuildError, EvaluatorConfig, IntegrationParams, ParametrizationConfig, RunTaskSpec,
    SamplerAggregatorConfig,
};
use crate::evaluation::{EvalBatchOptions, Evaluator, PointSpec};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::NaiveMonteCarloSamplerParams;
use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddIntegrationParams {
    pub evaluator: EvaluatorConfig,
    #[serde(default)]
    pub sampler_aggregator: Option<SamplerAggregatorConfig>,
    pub parametrization: ParametrizationConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddConfig {
    pub name: String,
    pub task_queue: Option<Vec<RunTaskSpec>>,
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
}

pub fn preprocess_run_add(mut config: RunAddConfig) -> Result<RunAddConfig, BuildError> {
    let resolved_sampler_aggregator = resolve_initial_sampler_aggregator(&config);
    let resolved_integration_params = IntegrationParams {
        evaluator: config.integration_params.evaluator.clone(),
        sampler_aggregator: resolved_sampler_aggregator.clone(),
        parametrization: config.integration_params.parametrization.clone(),
        evaluator_runner_params: config.integration_params.evaluator_runner_params.clone(),
        sampler_aggregator_runner_params: config
            .integration_params
            .sampler_aggregator_runner_params
            .clone(),
    };

    let mut evaluator = config.integration_params.evaluator.build()?;
    let point_spec = evaluator.get_point_spec();
    let evaluator_init_metadata = evaluator.get_init_metadata();
    config.point_spec = Some(point_spec.clone());

    let sampler_aggregator_init_metadata = preflight_compatibility(
        &config,
        &resolved_sampler_aggregator,
        &mut *evaluator,
        &point_spec,
    )?;
    config.resolved_integration_params = Some(resolved_integration_params);
    config.evaluator_init_metadata = Some(evaluator_init_metadata);
    config.sampler_aggregator_init_metadata = Some(sampler_aggregator_init_metadata);

    Ok(config)
}

fn resolve_initial_sampler_aggregator(config: &RunAddConfig) -> SamplerAggregatorConfig {
    config
        .task_queue
        .as_ref()
        .and_then(|tasks| {
            tasks.iter().find_map(|task| match task {
                RunTaskSpec::Sample {
                    sampler_aggregator, ..
                } => Some(sampler_aggregator.clone()),
                RunTaskSpec::Pause => None,
            })
        })
        .or_else(|| config.integration_params.sampler_aggregator.clone())
        .unwrap_or_else(|| SamplerAggregatorConfig::NaiveMonteCarlo {
            params: NaiveMonteCarloSamplerParams::default(),
        })
}

fn preflight_compatibility(
    config: &RunAddConfig,
    resolved_sampler_aggregator: &SamplerAggregatorConfig,
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
) -> Result<JsonValue, BuildError> {
    let mut sampler_aggregator = resolved_sampler_aggregator.build(point_spec.clone())?;
    sampler_aggregator.validate_point_spec(point_spec)?;
    let sampler_aggregator_init_metadata = sampler_aggregator.get_init_metadata();

    let mut parametrization = config.integration_params.parametrization.build()?;
    parametrization.validate_point_spec(point_spec)?;

    // One-point dry-run through sampler -> parametrization -> evaluator.
    let require_training_values = sampler_aggregator.training_samples_remaining().is_some();
    let latent_batch = sampler_aggregator
        .produce_latent_batch(1)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight failed to produce latent batch with sampler {}: {err}",
                resolved_sampler_aggregator.kind_str()
            ))
        })?
        .with_version(1);

    let transformed_batch = parametrization
        .materialize_batch(&latent_batch)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight parametrization materialization failed: {err}"
            ))
        })?;
    transformed_batch
        .validate_point_spec(point_spec)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight transformed batch has invalid point_spec: {err}"
            ))
        })?;

    let result = evaluator
        .eval_batch(
            &transformed_batch,
            EvalBatchOptions {
                require_training_values,
            },
        )
        .map_err(|err| {
            BuildError::incompatible(format!("preflight evaluator dry-run failed: {err}"))
        })?;

    if require_training_values {
        let training_weights = result.values.as_deref().ok_or_else(|| {
            BuildError::incompatible(
                "preflight evaluator dry-run omitted training values while sampler training is active",
            )
        })?;
        sampler_aggregator
            .ingest_training_weights(training_weights)
            .map_err(|err| {
                BuildError::incompatible(format!(
                    "preflight sampler ingest of evaluator training values failed: {err}"
                ))
            })?;
    }

    Ok(sampler_aggregator_init_metadata)
}
