use crate::core::{
    BuildError, IntoPreflightTask, ObservableConfig, ParametrizationConfig, ParametrizationState,
    RunTaskSpec, SamplerAggregatorConfig,
};
use crate::evaluation::{EvalBatchOptions, Evaluator, PointSpec};
use crate::sampling::{SamplerAggregatorSnapshot, StageHandoff};

pub(super) fn run_preflight(
    initial_sampler_aggregator: &SamplerAggregatorConfig,
    initial_parametrization: &ParametrizationConfig,
    resolved_tasks: &[RunTaskSpec],
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
) -> Result<Option<serde_json::Value>, BuildError> {
    let preflight_tasks = resolved_tasks
        .iter()
        .cloned()
        .map(IntoPreflightTask::into_preflight)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    if preflight_tasks.is_empty() {
        validate_initial_stage(
            initial_sampler_aggregator,
            initial_parametrization,
            point_spec,
        )?;
        return Ok(None);
    }

    let mut sampler_metadata = None::<serde_json::Value>;
    let mut handoff_snapshot = None;
    let mut parametrization_state: Option<ParametrizationState> = None;
    let mut current_observable = None;

    for task in preflight_tasks {
        if let (Some(sampler_aggregator), Some(parametrization)) =
            (task.sampler_config(), task.parametrization_config())
        {
            let task_observable = task
                .new_observable_config()?
                .or_else(|| current_observable.clone())
                .ok_or_else(|| {
                    BuildError::build(
                        "task requested observable reuse but no previous observable exists",
                    )
                })?;
            let previous_handoff_snapshot = handoff_snapshot.take();
            let previous_handoff = StageHandoff {
                sampler_snapshot: previous_handoff_snapshot.as_ref(),
                parametrization_snapshot: parametrization_state
                    .as_ref()
                    .map(|state| &state.snapshot),
                ..StageHandoff::default()
            };
            let metadata = preflight_single_stage(
                &task,
                task_observable.clone(),
                &sampler_aggregator,
                &parametrization,
                task.nr_expected_samples()
                    .and_then(|n| usize::try_from(n).ok()),
                evaluator,
                point_spec,
                Some(previous_handoff),
            )?;
            sampler_metadata.get_or_insert(metadata.0);
            handoff_snapshot = metadata.1;
            parametrization_state = metadata.2;
            current_observable = Some(task_observable);
        }
    }

    sampler_metadata.map(Some).ok_or_else(|| {
        BuildError::build("preflight produced no executable stage and no initial sampler metadata")
    })
}

fn validate_initial_stage(
    sampler_aggregator: &SamplerAggregatorConfig,
    parametrization_config: &ParametrizationConfig,
    point_spec: &PointSpec,
) -> Result<(), BuildError> {
    let sampler = sampler_aggregator.build(point_spec.clone(), None, None)?;
    sampler.validate_point_spec(point_spec)?;

    let parametrization = parametrization_config.build(None)?;
    parametrization.validate_point_spec(point_spec)?;
    Ok(())
}

fn preflight_single_stage(
    _task: &RunTaskSpec,
    task_observable: ObservableConfig,
    sampler_aggregator: &SamplerAggregatorConfig,
    parametrization_config: &ParametrizationConfig,
    sample_budget: Option<usize>,
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
    handoff: Option<StageHandoff<'_>>,
) -> Result<
    (
        serde_json::Value,
        Option<SamplerAggregatorSnapshot>,
        Option<ParametrizationState>,
    ),
    BuildError,
> {
    let mut sampler = sampler_aggregator.build(point_spec.clone(), sample_budget, handoff)?;
    sampler.validate_point_spec(point_spec)?;
    let sampler_metadata = sampler.get_init_metadata();

    let mut parametrization = parametrization_config.build(handoff)?;
    parametrization.validate_point_spec(point_spec)?;
    let parametrization_state = Some(ParametrizationState {
        config: parametrization_config.clone(),
        snapshot: parametrization.snapshot()?,
    });

    let require_training_values = sampler.training_samples_remaining().is_some();
    let latent_batch = sampler
        .produce_latent_batch(1)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight failed to produce latent batch with sampler {}: {err}",
                sampler_aggregator.kind_str()
            ))
        })?
        .with_observable_config(task_observable)
        .build();

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
            &latent_batch.observable,
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
        sampler
            .ingest_training_weights(training_weights)
            .map_err(|err| {
                BuildError::incompatible(format!(
                    "preflight sampler ingest of evaluator training values failed: {err}"
                ))
            })?;
    }

    let handoff_snapshot = Some(
        sampler
            .snapshot()
            .map_err(|err| BuildError::build(format!("preflight snapshot failed: {err}")))?,
    );
    Ok((sampler_metadata, handoff_snapshot, parametrization_state))
}
