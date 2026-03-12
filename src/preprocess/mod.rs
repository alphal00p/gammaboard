use crate::core::PointSpec;
use crate::engines::{BuildError, EvalBatchOptions, Evaluator, IntegrationParams};
use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize)]
pub struct RunAddConfig {
    pub name: String,
    pub pause_on_samples: Option<i64>,
    #[serde(flatten)]
    pub integration_params: IntegrationParams,
    pub target: Option<JsonValue>,
    #[serde(skip)]
    pub point_spec: Option<PointSpec>,
    #[serde(skip)]
    pub evaluator_init_metadata: Option<JsonValue>,
    #[serde(skip)]
    pub sampler_aggregator_init_metadata: Option<JsonValue>,
}

pub fn preprocess_run_add(mut config: RunAddConfig) -> Result<RunAddConfig, BuildError> {
    let mut evaluator = config.integration_params.evaluator.build()?;
    let point_spec = evaluator.get_point_spec();
    let evaluator_init_metadata = evaluator.get_init_metadata();
    config.point_spec = Some(point_spec.clone());

    let sampler_aggregator_init_metadata =
        preflight_compatibility(&config, &mut *evaluator, &point_spec)?;
    config.evaluator_init_metadata = Some(evaluator_init_metadata);
    config.sampler_aggregator_init_metadata = Some(sampler_aggregator_init_metadata);

    Ok(config)
}

fn preflight_compatibility(
    config: &RunAddConfig,
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
) -> Result<JsonValue, BuildError> {
    let mut sampler_aggregator = config
        .integration_params
        .sampler_aggregator
        .build(point_spec.clone())?;
    sampler_aggregator.validate_point_spec(point_spec)?;
    let sampler_aggregator_init_metadata = sampler_aggregator.get_init_metadata();

    let mut parametrization = config.integration_params.parametrization.build()?;
    parametrization.validate_point_spec(point_spec)?;

    // One-point dry-run through sampler -> parametrization -> evaluator.
    let require_training_values = sampler_aggregator.training_samples_remaining().is_some();
    let sample_batch = sampler_aggregator.produce_batch(1).map_err(|err| {
        BuildError::incompatible(format!(
            "preflight failed to produce sample batch with sampler {}: {err}",
            config.integration_params.sampler_aggregator.kind_str()
        ))
    })?;
    sample_batch
        .validate_point_spec(point_spec)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight sample batch has invalid point_spec: {err}"
            ))
        })?;

    let transformed_batch = parametrization
        .transform_batch(&sample_batch)
        .map_err(|err| {
            BuildError::incompatible(format!("preflight parametrization transform failed: {err}"))
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
