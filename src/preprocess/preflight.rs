use crate::core::{
    BuildError, IntoPreflightTask, ObservableConfig, ParametrizationConfig, ParametrizationState,
    RunTaskSpec, SamplerAggregatorConfig,
};
use crate::evaluation::{EvalBatchOptions, Evaluator, PointSpec};
use crate::sampling::{ParametrizationBuildContext, SamplerAggregatorSnapshot};

pub(super) fn run_preflight(
    base_observable: &ObservableConfig,
    initial_sampler_aggregator: &SamplerAggregatorConfig,
    initial_parametrization: &ParametrizationConfig,
    resolved_tasks: &[RunTaskSpec],
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
) -> Result<serde_json::Value, BuildError> {
    let preflight_tasks = resolved_tasks
        .iter()
        .cloned()
        .map(IntoPreflightTask::into_preflight)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    if preflight_tasks.is_empty() {
        let initial_task = RunTaskSpec::Sample {
            nr_samples: Some(1),
            sampler_aggregator: initial_sampler_aggregator.clone(),
            parametrization: initial_parametrization.clone(),
            observable: None,
        };
        return preflight_single_stage(
            &initial_task,
            base_observable.clone(),
            initial_sampler_aggregator,
            initial_parametrization,
            None,
            evaluator,
            point_spec,
            None,
            None,
        )
        .map(|(metadata, _, _)| metadata);
    }

    let mut sampler_metadata = None;
    let mut handoff_snapshot = None;
    let mut parametrization_state = None;
    let mut current_observable = base_observable.clone();

    for task in preflight_tasks {
        if let (Some(sampler_aggregator), Some(parametrization)) =
            (task.sampler_config(), task.parametrization_config())
        {
            let task_observable = task.observable_config(&current_observable);
            let metadata = preflight_single_stage(
                &task,
                task_observable.clone(),
                &sampler_aggregator,
                &parametrization,
                task.nr_expected_samples()
                    .and_then(|n| usize::try_from(n).ok()),
                evaluator,
                point_spec,
                handoff_snapshot.take(),
                parametrization_state.as_ref(),
            )?;
            sampler_metadata.get_or_insert(metadata.0);
            handoff_snapshot = metadata.1;
            parametrization_state = metadata.2;
            current_observable = task_observable;
        }
    }

    sampler_metadata.ok_or_else(|| {
        BuildError::build("preflight produced no executable stage and no initial sampler metadata")
    })
}

fn preflight_single_stage(
    _task: &RunTaskSpec,
    task_observable: ObservableConfig,
    sampler_aggregator: &SamplerAggregatorConfig,
    parametrization_config: &ParametrizationConfig,
    sample_budget: Option<usize>,
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
    handoff_snapshot: Option<SamplerAggregatorSnapshot>,
    previous_state: Option<&ParametrizationState>,
) -> Result<
    (
        serde_json::Value,
        Option<SamplerAggregatorSnapshot>,
        Option<ParametrizationState>,
    ),
    BuildError,
> {
    let handoff_snapshot_for_parametrization = handoff_snapshot.clone();
    let mut sampler = match handoff_snapshot {
        Some(snapshot) => sampler_aggregator.build_from_params_and_snapshot(
            point_spec.clone(),
            sample_budget,
            snapshot,
        ),
        None => sampler_aggregator.build(point_spec.clone(), sample_budget),
    }?;
    sampler.validate_point_spec(point_spec)?;
    let sampler_metadata = sampler.get_init_metadata();

    let mut parametrization = parametrization_config.build(ParametrizationBuildContext {
        sampler_aggregator_snapshot: handoff_snapshot_for_parametrization.as_ref(),
        parametrization_snapshot: previous_state.map(|state| &state.snapshot),
    })?;
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
