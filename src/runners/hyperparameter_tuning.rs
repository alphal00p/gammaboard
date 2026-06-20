use crate::core::{
    AccumulatorMetricName, AggregationStore, ControlPlaneStore, EgoboxInfillStrategy,
    EgoboxQeiStrategy, HyperparameterTuningAlgorithm, HyperparameterTuningObjectiveSpec,
    HyperparameterTuningParameterDomain, MeasurementMode, MeasurementQuantityName,
    MeasurementQuantitySpec, RunReadStore, RunSpecStore, RunTask, RunTaskSpec, RunTaskState,
    RunTaskStore, StoreError, TaskMeasurementOutput,
};
use crate::runners::controller_child::{
    ControllerChildRunRequest, choose_child_capacity, create_controller_child_run,
    list_child_runs_for_task, load_child_task_measurement,
    redistribute_parent_assignments_to_children,
};
use crate::runners::parameter_grid::{ParameterGridItem, cartesian_grid_len, cartesian_grid_point};
use egobox_ego::{EgorServiceBuilder, InfillStrategy, QEiStrategy, XType};
use ndarray::Array2;
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::{BTreeMap, BTreeSet};

const HYPERPARAMETER_TUNING_SPAWN_KIND: &str = "hyperparameter_tuning";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperparameterTrialOutput {
    pub index: usize,
    pub parameters: JsonValue,
    pub child_run_id: Option<i32>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_uncertainty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement: Option<TaskMeasurementOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperparameterTuningOutput {
    pub completed_trials: usize,
    pub running_trials: usize,
    pub failed_trials: usize,
    pub total_trials: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_trial: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_objective_value: Option<f64>,
    pub trials: Vec<HyperparameterTrialOutput>,
}

#[derive(Debug, Clone)]
struct OptimizerTrialCandidate {
    index: usize,
    parameters: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
struct OptimizerObservation {
    parameters: BTreeMap<String, toml::Value>,
    objective_value: f64,
}

#[derive(Debug, Clone)]
struct PreviousTrial {
    index: usize,
    status: String,
    parameters: BTreeMap<String, toml::Value>,
    objective_value: Option<f64>,
}

#[derive(Debug, Clone)]
struct OptimizerTrialPlan {
    total_trials: usize,
    candidates: Vec<OptimizerTrialCandidate>,
}

pub struct HyperparameterTuningRunner<S> {
    store: S,
    run_id: i32,
    task: RunTask,
}

impl<S> HyperparameterTuningRunner<S> {
    pub fn new(store: S, run_id: i32, task: RunTask) -> Self {
        Self {
            store,
            run_id,
            task,
        }
    }
}

impl<S> HyperparameterTuningRunner<S>
where
    S: AggregationStore
        + ControlPlaneStore
        + RunReadStore
        + RunSpecStore
        + RunTaskStore
        + Send
        + Sync,
{
    pub async fn tick(&mut self) -> Result<bool, StoreError> {
        let RunTaskSpec::HyperparameterTuning {
            optimizer,
            objective,
            parameters,
            trial_run_toml,
            max_concurrent_trials,
        } = &self.task.task
        else {
            return Err(StoreError::store(
                "hyperparameter tuning runner got non-tuning task",
            ));
        };

        let child_runs = list_child_runs_for_task(
            &self.store,
            self.run_id,
            self.task.id,
            HYPERPARAMETER_TUNING_SPAWN_KIND,
        )
        .await?;
        let child_runs_by_label = child_runs
            .iter()
            .filter_map(|run| run.spawn_label.as_deref().map(|label| (label, run)))
            .collect::<BTreeMap<_, _>>();

        let existing_trial_indices = child_runs_by_label
            .keys()
            .filter_map(|label| label.parse::<usize>().ok())
            .collect::<BTreeSet<_>>();
        let previous_trials = previous_trial_parameters(self.task.controller_output.as_ref())?;
        let previous_observations =
            previous_trial_observations(self.task.controller_output.as_ref())?;
        let previous_running_count = previous_trials
            .iter()
            .filter(|(_, trial)| {
                child_runs_by_label.contains_key(trial.index.to_string().as_str())
                    && trial.status != "completed"
                    && trial.status != "failed"
            })
            .count();
        let candidate_capacity =
            choose_child_capacity(*max_concurrent_trials, previous_running_count);
        let optimizer_plan = plan_optimizer_trials(
            optimizer,
            parameters,
            &existing_trial_indices,
            &previous_trials,
            &previous_observations,
            objective.mode,
            candidate_capacity,
        )?;
        let total_trials = optimizer_plan.total_trials;
        let candidates_by_index = optimizer_plan
            .candidates
            .iter()
            .map(|candidate| (candidate.index, candidate))
            .collect::<BTreeMap<_, _>>();
        let mut trials = Vec::with_capacity(total_trials);
        let mut completed_count = 0usize;
        let mut running_count = 0usize;
        let mut failed_count = 0usize;

        for index in candidates_by_index.keys().copied().collect::<Vec<_>>() {
            let label = index.to_string();
            let values = candidates_by_index
                .get(&index)
                .ok_or_else(|| StoreError::store(format!("optimizer did not plan trial {index}")))?
                .parameters
                .clone();
            let parameters_json = parameters_to_json(&values)?;
            let child = child_runs_by_label.get(label.as_str()).copied();

            let Some(child) = child else {
                trials.push(HyperparameterTrialOutput {
                    index,
                    parameters: parameters_json,
                    child_run_id: None,
                    status: "planned".to_string(),
                    objective_value: None,
                    objective_uncertainty: None,
                    measurement: None,
                    failure_reason: None,
                });
                continue;
            };

            let measurement_output =
                load_child_task_measurement(&self.store, child.run_id, &objective.source_task)
                    .await?;
            match measurement_output.output {
                Some(TaskMeasurementOutput::Completed { results }) => {
                    match objective_result(objective, &results) {
                        Ok(result) => {
                            completed_count += 1;
                            trials.push(HyperparameterTrialOutput {
                                index,
                                parameters: parameters_json,
                                child_run_id: Some(child.run_id),
                                status: "completed".to_string(),
                                objective_value: Some(result.value),
                                objective_uncertainty: result.uncertainty,
                                measurement: Some(TaskMeasurementOutput::Completed { results }),
                                failure_reason: None,
                            });
                        }
                        Err(reason) => {
                            failed_count += 1;
                            let failure_reason = objective_failure_reason(
                                index,
                                child.run_id,
                                objective,
                                &results,
                                &reason,
                            );
                            trials.push(HyperparameterTrialOutput {
                                index,
                                parameters: parameters_json,
                                child_run_id: Some(child.run_id),
                                status: "failed".to_string(),
                                objective_value: None,
                                objective_uncertainty: None,
                                measurement: Some(TaskMeasurementOutput::Completed { results }),
                                failure_reason: Some(failure_reason),
                            });
                        }
                    }
                }
                Some(TaskMeasurementOutput::Failed { reason }) => {
                    failed_count += 1;
                    let failure_reason = objective_measurement_failure_reason(
                        index,
                        child.run_id,
                        objective,
                        &reason,
                    );
                    trials.push(HyperparameterTrialOutput {
                        index,
                        parameters: parameters_json,
                        child_run_id: Some(child.run_id),
                        status: "failed".to_string(),
                        objective_value: None,
                        objective_uncertainty: None,
                        measurement: Some(TaskMeasurementOutput::Failed {
                            reason: reason.clone(),
                        }),
                        failure_reason: Some(failure_reason),
                    });
                }
                None => {
                    if measurement_output.task_state == RunTaskState::Completed {
                        failed_count += 1;
                        let failure_reason =
                            objective_missing_measurement_reason(index, child.run_id, objective);
                        trials.push(HyperparameterTrialOutput {
                            index,
                            parameters: parameters_json,
                            child_run_id: Some(child.run_id),
                            status: "failed".to_string(),
                            objective_value: None,
                            objective_uncertainty: None,
                            measurement: None,
                            failure_reason: Some(failure_reason),
                        });
                    } else {
                        running_count += 1;
                        trials.push(HyperparameterTrialOutput {
                            index,
                            parameters: parameters_json,
                            child_run_id: Some(child.run_id),
                            status: measurement_output.task_state.as_str().to_string(),
                            objective_value: None,
                            objective_uncertainty: None,
                            measurement: None,
                            failure_reason: None,
                        });
                    }
                }
            }
        }

        if failed_count > 0 {
            self.persist_output(
                total_trials,
                completed_count,
                running_count,
                failed_count,
                trials,
            )
            .await?;
            redistribute_parent_assignments_to_children(&self.store, self.run_id, []).await?;
            self.store
                .fail_run_task(
                    self.task.id,
                    &format!("hyperparameter tuning failed: failed_trials={failed_count}"),
                )
                .await?;
            return Ok(true);
        }

        if completed_count + failed_count == total_trials {
            self.persist_output(
                total_trials,
                completed_count,
                running_count,
                failed_count,
                trials,
            )
            .await?;
            self.store
                .update_run_task_progress(self.task.id, total_trials as i64, completed_count as i64)
                .await?;
            redistribute_parent_assignments_to_children(&self.store, self.run_id, []).await?;
            self.store.complete_run_task(self.task.id).await?;
            return Ok(true);
        }

        let mut created_child_run_ids = Vec::new();
        let mut capacity = choose_child_capacity(*max_concurrent_trials, running_count);
        if capacity > 0 {
            for index in candidates_by_index.keys().copied().collect::<Vec<_>>() {
                if capacity == 0 {
                    break;
                }
                let label = index.to_string();
                if child_runs_by_label.contains_key(label.as_str()) {
                    continue;
                }
                let replacements = candidates_by_index
                    .get(&index)
                    .ok_or_else(|| {
                        StoreError::store(format!("optimizer did not plan trial {index}"))
                    })?
                    .parameters
                    .clone();
                let child = match create_controller_child_run(
                    &self.store,
                    ControllerChildRunRequest {
                        parent_run_id: self.run_id,
                        parent_task_id: self.task.id,
                        spawn_kind: HYPERPARAMETER_TUNING_SPAWN_KIND.to_string(),
                        spawn_label: label,
                        run_toml: trial_run_toml.clone(),
                        replacements,
                    },
                )
                .await
                {
                    Ok(child) => child,
                    Err(err) => {
                        let reason = format!("failed to create tuning trial {index}: {err}");
                        self.persist_output(
                            total_trials,
                            completed_count,
                            running_count,
                            failed_count,
                            trials,
                        )
                        .await?;
                        self.store.fail_run_task(self.task.id, &reason).await?;
                        return Ok(true);
                    }
                };
                created_child_run_ids.push(child.run_id);
                capacity -= 1;
            }
        }

        let mut runnable_child_run_ids = trials
            .iter()
            .filter(|trial| trial.status != "completed" && trial.status != "failed")
            .filter_map(|trial| trial.child_run_id)
            .collect::<Vec<_>>();
        runnable_child_run_ids.extend(created_child_run_ids);
        redistribute_parent_assignments_to_children(
            &self.store,
            self.run_id,
            runnable_child_run_ids,
        )
        .await?;

        self.store
            .update_run_task_progress(self.task.id, total_trials as i64, completed_count as i64)
            .await?;
        self.persist_output(
            total_trials,
            completed_count,
            running_count,
            failed_count,
            trials,
        )
        .await?;
        Ok(false)
    }

    async fn persist_output(
        &self,
        total_trials: usize,
        completed_trials: usize,
        running_trials: usize,
        failed_trials: usize,
        trials: Vec<HyperparameterTrialOutput>,
    ) -> Result<(), StoreError> {
        let (best_trial, best_objective_value) =
            best_trial(&trials, objective_mode(&self.task.task));
        let output = serde_json::to_value(HyperparameterTuningOutput {
            total_trials,
            completed_trials,
            running_trials,
            failed_trials,
            best_trial,
            best_objective_value,
            trials,
        })
        .map_err(|err| StoreError::store(format!("failed to serialize tuning output: {err}")))?;
        self.store
            .persist_task_controller_output(self.task.id, &output)
            .await
    }
}

fn objective_mode(task: &RunTaskSpec) -> MeasurementMode {
    match task {
        RunTaskSpec::HyperparameterTuning { objective, .. } => objective.mode,
        _ => MeasurementMode::Minimize,
    }
}

fn objective_result<'a>(
    objective: &HyperparameterTuningObjectiveSpec,
    results: &'a [crate::core::MeasurementResult],
) -> Result<&'a crate::core::MeasurementResult, String> {
    let selector = objective_selector(objective);
    let matches = results
        .iter()
        .filter(|result| {
            result.name == selector.0
                && match (&selector.1, &result.component) {
                    (Some(expected), Some(actual)) => expected == actual,
                    (Some(_), None) => false,
                    (None, _) => true,
                }
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [result] => Ok(result),
        [] => Err(format!(
            "objective measurement did not produce metric {:?}{}",
            selector.0,
            selector
                .1
                .as_ref()
                .map(|component| format!(" component {component:?}"))
                .unwrap_or_default()
        )),
        _ => Err(format!(
            "objective measurement produced multiple {:?} results; set objective.quantity.component or objective.metric.component",
            selector.0
        )),
    }
}

fn objective_failure_reason(
    trial_index: usize,
    child_run_id: i32,
    objective: &HyperparameterTuningObjectiveSpec,
    results: &[crate::core::MeasurementResult],
    reason: &str,
) -> String {
    format!(
        "trial {trial_index} child_run_id={child_run_id} objective source_task={} requested={} failed: {reason}; available_results=[{}]",
        objective.source_task,
        objective_selector_label(objective),
        available_measurement_results_label(results),
    )
}

fn objective_measurement_failure_reason(
    trial_index: usize,
    child_run_id: i32,
    objective: &HyperparameterTuningObjectiveSpec,
    reason: &str,
) -> String {
    format!(
        "trial {trial_index} child_run_id={child_run_id} objective source_task={} requested={} measurement failed: {reason}",
        objective.source_task,
        objective_selector_label(objective),
    )
}

fn objective_missing_measurement_reason(
    trial_index: usize,
    child_run_id: i32,
    objective: &HyperparameterTuningObjectiveSpec,
) -> String {
    format!(
        "trial {trial_index} child_run_id={child_run_id} objective source_task={} requested={} failed: source task completed without measurement output",
        objective.source_task,
        objective_selector_label(objective),
    )
}

fn objective_selector_label(objective: &HyperparameterTuningObjectiveSpec) -> String {
    let (name, component) = objective_selector(objective);
    metric_selector_label(name, component.as_deref())
}

fn available_measurement_results_label(results: &[crate::core::MeasurementResult]) -> String {
    if results.is_empty() {
        return "none".to_string();
    }
    results
        .iter()
        .map(|result| metric_selector_label(result.name, result.component.as_deref()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn metric_selector_label(name: AccumulatorMetricName, component: Option<&str>) -> String {
    match component {
        Some(component) => format!("{name:?}(component={component})"),
        None => format!("{name:?}"),
    }
}

fn objective_selector(
    objective: &HyperparameterTuningObjectiveSpec,
) -> (AccumulatorMetricName, Option<String>) {
    if let Some(metric) = objective.metric.as_ref() {
        let selector = metric.selector();
        return (selector.name, selector.component);
    }
    match &objective.quantity {
        MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue) => {
            (AccumulatorMetricName::Mean, None)
        }
        MeasurementQuantitySpec::Metric(metric) => {
            let selector = metric.selector();
            (selector.name, selector.component)
        }
    }
}

fn best_trial(
    trials: &[HyperparameterTrialOutput],
    mode: MeasurementMode,
) -> (Option<usize>, Option<f64>) {
    let mut best: Option<(usize, f64)> = None;
    for trial in trials {
        let Some(value) = trial.objective_value else {
            continue;
        };
        let is_better = best
            .map(|(_, current)| match mode {
                MeasurementMode::Minimize => value < current,
                MeasurementMode::Maximize => value > current,
            })
            .unwrap_or(true);
        if is_better {
            best = Some((trial.index, value));
        }
    }
    best.map_or((None, None), |(index, value)| (Some(index), Some(value)))
}

#[cfg(test)]
fn optimizer_trial_count(
    optimizer: &crate::core::HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
) -> Result<usize, StoreError> {
    Ok(plan_optimizer_trials(
        optimizer,
        parameters,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &[],
        MeasurementMode::Minimize,
        usize::MAX,
    )?
    .total_trials)
}

fn plan_optimizer_trials(
    optimizer: &crate::core::HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    existing_trial_indices: &BTreeSet<usize>,
    previous_trials: &BTreeMap<usize, PreviousTrial>,
    observations: &[OptimizerObservation],
    mode: MeasurementMode,
    candidate_capacity: usize,
) -> Result<OptimizerTrialPlan, StoreError> {
    match optimizer.algorithm {
        HyperparameterTuningAlgorithm::RandomSearch => {
            let total_trials = optimizer
                .random_search_params()
                .map_err(StoreError::store)?
                .max_trials;
            let candidates = (0..total_trials)
                .map(|index| {
                    Ok(OptimizerTrialCandidate {
                        index,
                        parameters: random_trial_parameters(optimizer, parameters, index)?,
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            Ok(OptimizerTrialPlan {
                total_trials,
                candidates,
            })
        }
        HyperparameterTuningAlgorithm::GridSearch => {
            let grid_items = hyperparameter_grid_items(parameters)?;
            let total_trials = cartesian_grid_len(&grid_items)?;
            let candidates = (0..total_trials)
                .map(|index| {
                    Ok(OptimizerTrialCandidate {
                        index,
                        parameters: cartesian_grid_point(&grid_items, index)?,
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            Ok(OptimizerTrialPlan {
                total_trials,
                candidates,
            })
        }
        HyperparameterTuningAlgorithm::Egobox => egobox_trial_plan(
            optimizer,
            parameters,
            existing_trial_indices,
            previous_trials,
            observations,
            mode,
            candidate_capacity,
        ),
    }
}

#[cfg(test)]
fn trial_parameters(
    optimizer: &crate::core::HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    index: usize,
) -> Result<BTreeMap<String, toml::Value>, StoreError> {
    plan_optimizer_trials(
        optimizer,
        parameters,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &[],
        MeasurementMode::Minimize,
        usize::MAX,
    )?
    .candidates
    .into_iter()
    .find(|candidate| candidate.index == index)
    .map(|candidate| candidate.parameters)
    .ok_or_else(|| StoreError::store(format!("optimizer did not plan trial {index}")))
}

fn random_trial_parameters(
    optimizer: &crate::core::HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    index: usize,
) -> Result<BTreeMap<String, toml::Value>, StoreError> {
    let params = optimizer
        .random_search_params()
        .map_err(StoreError::store)?;
    let mut rng =
        Xoshiro256StarStar::seed_from_u64(params.seed.unwrap_or(0) ^ splitmix64(index as u64));
    parameters
        .iter()
        .map(|(name, domain)| Ok((name.clone(), sample_parameter(domain, &mut rng)?)))
        .collect()
}

fn sample_parameter(
    domain: &HyperparameterTuningParameterDomain,
    rng: &mut Xoshiro256StarStar,
) -> Result<toml::Value, StoreError> {
    match domain {
        HyperparameterTuningParameterDomain::Float(domain) => {
            Ok(toml::Value::Float(rng.random_range(domain.min..domain.max)))
        }
        HyperparameterTuningParameterDomain::Integer(domain) => {
            let step = domain.step.unwrap_or(1);
            let span = domain.max.checked_sub(domain.min).ok_or_else(|| {
                StoreError::store("integer parameter domain bounds overflow while sampling")
            })?;
            let count = (span / step) + 1;
            if count <= 0 {
                return Err(StoreError::store("integer parameter domain is empty"));
            }
            let offset = rng.random_range(0..count);
            Ok(toml::Value::Integer(domain.min + offset * step))
        }
        HyperparameterTuningParameterDomain::Categorical(domain) => {
            let values = categorical_domain_values(domain)?;
            let index = rng.random_range(0..values.len());
            Ok(values[index].clone())
        }
    }
}

fn hyperparameter_grid_items(
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
) -> Result<Vec<ParameterGridItem>, StoreError> {
    parameters
        .iter()
        .map(|(name, domain)| {
            Ok(ParameterGridItem {
                name: name.clone(),
                values: grid_parameter_values(domain)?,
            })
        })
        .collect()
}

fn grid_parameter_values(
    domain: &HyperparameterTuningParameterDomain,
) -> Result<Vec<toml::Value>, StoreError> {
    match domain {
        HyperparameterTuningParameterDomain::Integer(domain) => {
            let step = domain.step.unwrap_or(1);
            let mut values = Vec::new();
            let mut value = domain.min;
            while value <= domain.max {
                values.push(toml::Value::Integer(value));
                match value.checked_add(step) {
                    Some(next) => value = next,
                    None => break,
                }
            }
            Ok(values)
        }
        HyperparameterTuningParameterDomain::Categorical(domain) => {
            categorical_domain_values(domain)
        }
        HyperparameterTuningParameterDomain::Float(_) => Err(StoreError::store(
            "grid_search does not support float parameter domains yet; use integer/categorical domains",
        )),
    }
}

fn egobox_trial_plan(
    optimizer: &crate::core::HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    existing_trial_indices: &BTreeSet<usize>,
    previous_trials: &BTreeMap<usize, PreviousTrial>,
    observations: &[OptimizerObservation],
    mode: MeasurementMode,
    candidate_capacity: usize,
) -> Result<OptimizerTrialPlan, StoreError> {
    let params = optimizer.egobox_params().map_err(StoreError::store)?;
    let total_trials = params.max_trials;
    let mut candidates = previous_trials
        .values()
        .filter(|trial| trial.index < total_trials)
        .map(|trial| OptimizerTrialCandidate {
            index: trial.index,
            parameters: trial.parameters.clone(),
        })
        .collect::<Vec<_>>();

    let planned_new = total_trials.saturating_sub(existing_trial_indices.len());
    let desired_new = candidate_capacity
        .min(params.parallel_candidates)
        .min(planned_new);
    if desired_new == 0 {
        return Ok(OptimizerTrialPlan {
            total_trials,
            candidates,
        });
    }

    let encoder = ParameterEncoder::new(parameters)?;
    let next_indices = (0..total_trials)
        .filter(|index| !existing_trial_indices.contains(index))
        .take(desired_new)
        .collect::<Vec<_>>();
    let new_candidates =
        if observations.len() < effective_egobox_initial_design(&params, parameters.len()) {
            next_indices
                .into_iter()
                .map(|index| {
                    let mut rng = Xoshiro256StarStar::seed_from_u64(
                        params.seed.unwrap_or(0) ^ splitmix64(index as u64),
                    );
                    Ok(OptimizerTrialCandidate {
                        index,
                        parameters: parameters
                            .iter()
                            .map(|(name, domain)| {
                                Ok((name.clone(), sample_parameter(domain, &mut rng)?))
                            })
                            .collect::<Result<BTreeMap<_, _>, StoreError>>()?,
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?
        } else {
            suggest_egobox_candidates(
                &params,
                &encoder,
                observations,
                mode,
                &next_indices,
                existing_trial_indices,
                previous_trials,
            )?
        };
    candidates.extend(new_candidates);
    candidates.sort_by_key(|candidate| candidate.index);
    Ok(OptimizerTrialPlan {
        total_trials,
        candidates,
    })
}

fn effective_egobox_initial_design(
    params: &crate::core::EgoboxOptimizerParams,
    parameter_count: usize,
) -> usize {
    params.initial_design.max(parameter_count + 1).max(2)
}

fn suggest_egobox_candidates(
    params: &crate::core::EgoboxOptimizerParams,
    encoder: &ParameterEncoder,
    observations: &[OptimizerObservation],
    mode: MeasurementMode,
    next_indices: &[usize],
    existing_trial_indices: &BTreeSet<usize>,
    previous_trials: &BTreeMap<usize, PreviousTrial>,
) -> Result<Vec<OptimizerTrialCandidate>, StoreError> {
    let x_data = encoder.observation_x_data(observations)?;
    let y_values = observations
        .iter()
        .map(|observation| match mode {
            MeasurementMode::Minimize => observation.objective_value,
            MeasurementMode::Maximize => -observation.objective_value,
        })
        .collect::<Vec<_>>();
    let y_data = Array2::from_shape_vec((y_values.len(), 1), y_values)
        .map_err(|err| StoreError::store(format!("failed to build egobox y_data: {err}")))?;
    let infill_strategy = params
        .infill
        .map(InfillStrategy::from)
        .unwrap_or(InfillStrategy::EI);
    let qei_strategy = params.qei_strategy.map(QEiStrategy::from);
    let builder = EgorServiceBuilder::optimize().configure(|config| {
        let config = if let Some(seed) = params.seed {
            config.seed(seed)
        } else {
            config
        };
        let config = config
            .n_doe(effective_egobox_initial_design(params, encoder.len()))
            .configure_qei(|qei| {
                let mut qei = qei.batch(next_indices.len().max(1));
                if let Some(strategy) = qei_strategy {
                    qei = qei.strategy(strategy);
                }
                if let Some(optmod) = params.qei_optmod {
                    qei.optmod(optmod)
                } else {
                    qei
                }
            });
        let config = if let Some(n_start) = params.n_start {
            config.n_start(n_start)
        } else {
            config
        };
        config.infill_strategy(infill_strategy)
    });
    let service = builder
        .min_within_mixint_space(&encoder.xtypes)
        .map_err(|err| StoreError::store(format!("failed to configure egobox optimizer: {err}")))?;
    let suggested = service.suggest(&x_data, &y_data);
    let mut existing_encoded =
        existing_encoded_points(encoder, existing_trial_indices, previous_trials)?;
    let mut candidates = Vec::new();
    for (offset, index) in next_indices.iter().copied().enumerate() {
        let row_index = offset.min(suggested.nrows().saturating_sub(1));
        let encoded = suggested.row(row_index).to_vec();
        let parameters = if contains_encoded(&existing_encoded, &encoded) {
            let mut rng = Xoshiro256StarStar::seed_from_u64(
                params.seed.unwrap_or(0) ^ splitmix64(index as u64),
            );
            encoder.random_distinct_point(&mut rng, &existing_encoded)?
        } else {
            encoder.decode(&encoded)?
        };
        existing_encoded.push(encoder.encode(&parameters)?);
        candidates.push(OptimizerTrialCandidate { index, parameters });
    }
    Ok(candidates)
}

impl From<EgoboxInfillStrategy> for InfillStrategy {
    fn from(value: EgoboxInfillStrategy) -> Self {
        match value {
            EgoboxInfillStrategy::Ei => Self::EI,
            EgoboxInfillStrategy::LogEi => Self::LogEI,
            EgoboxInfillStrategy::Wb2 => Self::WB2,
            EgoboxInfillStrategy::Wb2s => Self::WB2S,
        }
    }
}

impl From<EgoboxQeiStrategy> for QEiStrategy {
    fn from(value: EgoboxQeiStrategy) -> Self {
        match value {
            EgoboxQeiStrategy::KrigingBeliever => Self::KrigingBeliever,
            EgoboxQeiStrategy::KrigingBelieverLowerBound => Self::KrigingBelieverLowerBound,
            EgoboxQeiStrategy::KrigingBelieverUpperBound => Self::KrigingBelieverUpperBound,
            EgoboxQeiStrategy::ConstantLiarMinimum => Self::ConstantLiarMinimum,
        }
    }
}

#[derive(Debug, Clone)]
struct ParameterEncoder {
    names: Vec<String>,
    domains: Vec<HyperparameterTuningParameterDomain>,
    xtypes: Vec<XType>,
}

impl ParameterEncoder {
    fn new(
        parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    ) -> Result<Self, StoreError> {
        let mut names = Vec::new();
        let mut domains = Vec::new();
        let mut xtypes = Vec::new();
        for (name, domain) in parameters {
            names.push(name.clone());
            domains.push(domain.clone());
            xtypes.push(domain_to_egobox_xtype(name, domain)?);
        }
        Ok(Self {
            names,
            domains,
            xtypes,
        })
    }

    fn encode(&self, parameters: &BTreeMap<String, toml::Value>) -> Result<Vec<f64>, StoreError> {
        self.names
            .iter()
            .zip(&self.domains)
            .map(|(name, domain)| {
                let value = parameters.get(name).ok_or_else(|| {
                    StoreError::store(format!("egobox trial is missing parameter {name:?}"))
                })?;
                encode_parameter_value(name, domain, value)
            })
            .collect()
    }

    fn decode(&self, encoded: &[f64]) -> Result<BTreeMap<String, toml::Value>, StoreError> {
        if encoded.len() != self.names.len() {
            return Err(StoreError::store(format!(
                "egobox suggested {} parameters, expected {}",
                encoded.len(),
                self.names.len()
            )));
        }
        self.names
            .iter()
            .zip(&self.domains)
            .zip(encoded.iter().copied())
            .map(|((name, domain), value)| {
                Ok((name.clone(), decode_parameter_value(name, domain, value)?))
            })
            .collect()
    }

    fn random_point(
        &self,
        rng: &mut Xoshiro256StarStar,
    ) -> Result<BTreeMap<String, toml::Value>, StoreError> {
        self.names
            .iter()
            .zip(&self.domains)
            .map(|(name, domain)| Ok((name.clone(), sample_parameter(domain, rng)?)))
            .collect()
    }

    fn random_distinct_point(
        &self,
        rng: &mut Xoshiro256StarStar,
        existing: &[Vec<f64>],
    ) -> Result<BTreeMap<String, toml::Value>, StoreError> {
        let mut last = None;
        for _ in 0..64 {
            let point = self.random_point(rng)?;
            let encoded = self.encode(&point)?;
            if !contains_encoded(existing, &encoded) {
                return Ok(point);
            }
            last = Some(point);
        }
        if let Some(point) = self.first_distinct_finite_point(existing)? {
            return Ok(point);
        }
        last.ok_or_else(|| StoreError::store("failed to sample egobox fallback point"))
    }

    fn len(&self) -> usize {
        self.names.len()
    }

    fn first_distinct_finite_point(
        &self,
        existing: &[Vec<f64>],
    ) -> Result<Option<BTreeMap<String, toml::Value>>, StoreError> {
        let mut values = Vec::new();
        for domain in &self.domains {
            let domain_values = match domain {
                HyperparameterTuningParameterDomain::Float(_) => return Ok(None),
                HyperparameterTuningParameterDomain::Integer(domain) => {
                    integer_domain_values(domain)?
                        .into_iter()
                        .map(toml::Value::Integer)
                        .collect()
                }
                HyperparameterTuningParameterDomain::Categorical(domain) => {
                    categorical_domain_values(domain)?
                }
            };
            values.push(domain_values);
        }
        let mut point = BTreeMap::new();
        self.find_distinct_finite_point(0, &values, &mut point, existing)
    }

    fn find_distinct_finite_point(
        &self,
        depth: usize,
        values: &[Vec<toml::Value>],
        point: &mut BTreeMap<String, toml::Value>,
        existing: &[Vec<f64>],
    ) -> Result<Option<BTreeMap<String, toml::Value>>, StoreError> {
        if depth == self.names.len() {
            return Ok((!contains_encoded(existing, &self.encode(point)?)).then(|| point.clone()));
        }
        let name = &self.names[depth];
        for value in &values[depth] {
            point.insert(name.clone(), value.clone());
            if let Some(candidate) =
                self.find_distinct_finite_point(depth + 1, values, point, existing)?
            {
                return Ok(Some(candidate));
            }
        }
        point.remove(name);
        Ok(None)
    }

    fn observation_x_data(
        &self,
        observations: &[OptimizerObservation],
    ) -> Result<Array2<f64>, StoreError> {
        let mut data = Vec::with_capacity(observations.len() * self.names.len());
        for observation in observations {
            data.extend(self.encode(&observation.parameters)?);
        }
        Array2::from_shape_vec((observations.len(), self.names.len()), data)
            .map_err(|err| StoreError::store(format!("failed to build egobox x_data: {err}")))
    }
}

fn domain_to_egobox_xtype(
    name: &str,
    domain: &HyperparameterTuningParameterDomain,
) -> Result<XType, StoreError> {
    match domain {
        HyperparameterTuningParameterDomain::Float(domain) => {
            Ok(XType::Float(domain.min, domain.max))
        }
        HyperparameterTuningParameterDomain::Integer(domain) => {
            let step = domain.step.unwrap_or(1);
            if step == 1 && i32::try_from(domain.min).is_ok() && i32::try_from(domain.max).is_ok() {
                Ok(XType::Int(domain.min as i32, domain.max as i32))
            } else {
                Ok(XType::Ord(
                    integer_domain_values(domain)?
                        .into_iter()
                        .map(|value| value as f64)
                        .collect(),
                ))
            }
        }
        HyperparameterTuningParameterDomain::Categorical(domain) => {
            let values = categorical_domain_values(domain)?;
            if values.len() < 2 {
                return Err(StoreError::store(format!(
                    "egobox categorical parameter {name:?} needs at least two values"
                )));
            }
            Ok(XType::Enum(values.len()))
        }
    }
}

fn encode_parameter_value(
    name: &str,
    domain: &HyperparameterTuningParameterDomain,
    value: &toml::Value,
) -> Result<f64, StoreError> {
    match domain {
        HyperparameterTuningParameterDomain::Float(_) => value
            .as_float()
            .or_else(|| {
                value
                    .as_integer()
                    .and_then(|integer| integer.to_string().parse::<f64>().ok())
            })
            .ok_or_else(|| {
                StoreError::store(format!("egobox float parameter {name:?} must be numeric"))
            }),
        HyperparameterTuningParameterDomain::Integer(_) => value
            .as_integer()
            .and_then(|integer| integer.to_string().parse::<f64>().ok())
            .ok_or_else(|| {
                StoreError::store(format!("egobox integer parameter {name:?} must be integer"))
            }),
        HyperparameterTuningParameterDomain::Categorical(domain) => {
            categorical_domain_values(domain)?
                .iter()
                .position(|candidate| candidate == value)
                .and_then(|index| index.to_string().parse::<f64>().ok())
                .ok_or_else(|| {
                    StoreError::store(format!(
                        "egobox categorical parameter {name:?} has value outside its domain"
                    ))
                })
        }
    }
}

fn decode_parameter_value(
    _name: &str,
    domain: &HyperparameterTuningParameterDomain,
    value: f64,
) -> Result<toml::Value, StoreError> {
    match domain {
        HyperparameterTuningParameterDomain::Float(domain) => {
            Ok(toml::Value::Float(value.clamp(domain.min, domain.max)))
        }
        HyperparameterTuningParameterDomain::Integer(domain) => {
            let values = integer_domain_values(domain)?;
            let rounded = take_closest_i64(&values, value);
            Ok(toml::Value::Integer(rounded))
        }
        HyperparameterTuningParameterDomain::Categorical(domain) => {
            let values = categorical_domain_values(domain)?;
            let max_index = values.len().saturating_sub(1);
            let index = value.round().clamp(0.0, max_index as f64) as usize;
            Ok(values[index].clone())
        }
    }
}

fn categorical_domain_values(
    domain: &crate::core::HyperparameterTuningCategoricalDomain,
) -> Result<Vec<toml::Value>, StoreError> {
    domain
        .source
        .values("hyperparameter_tuning.parameters")
        .map_err(StoreError::store)
}

fn integer_domain_values(
    domain: &crate::core::HyperparameterTuningIntegerDomain,
) -> Result<Vec<i64>, StoreError> {
    let step = domain.step.unwrap_or(1);
    let mut values = Vec::new();
    let mut value = domain.min;
    while value <= domain.max {
        values.push(value);
        match value.checked_add(step) {
            Some(next) => value = next,
            None => break,
        }
    }
    if values.is_empty() {
        return Err(StoreError::store("integer parameter domain is empty"));
    }
    Ok(values)
}

fn take_closest_i64(values: &[i64], value: f64) -> i64 {
    values
        .iter()
        .copied()
        .min_by(|left, right| {
            let left_dist = ((*left as f64) - value).abs();
            let right_dist = ((*right as f64) - value).abs();
            left_dist
                .partial_cmp(&right_dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0)
}

fn existing_encoded_points(
    encoder: &ParameterEncoder,
    existing_trial_indices: &BTreeSet<usize>,
    previous_trials: &BTreeMap<usize, PreviousTrial>,
) -> Result<Vec<Vec<f64>>, StoreError> {
    existing_trial_indices
        .iter()
        .filter_map(|index| previous_trials.get(index))
        .map(|trial| encoder.encode(&trial.parameters))
        .collect()
}

fn contains_encoded(existing: &[Vec<f64>], candidate: &[f64]) -> bool {
    existing.iter().any(|point| {
        point.len() == candidate.len()
            && point
                .iter()
                .zip(candidate)
                .all(|(left, right)| (left - right).abs() < 1e-9)
    })
}

fn previous_trial_parameters(
    output: Option<&JsonValue>,
) -> Result<BTreeMap<usize, PreviousTrial>, StoreError> {
    let Some(trials) = output
        .and_then(|output| output.get("trials"))
        .and_then(JsonValue::as_array)
    else {
        return Ok(BTreeMap::new());
    };
    trials
        .iter()
        .filter_map(|trial| {
            let index = trial.get("index").and_then(JsonValue::as_u64)? as usize;
            let status = trial
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("planned")
                .to_string();
            let parameters = trial.get("parameters")?.as_object()?;
            let parameters = parameters
                .iter()
                .map(|(name, value)| Ok((name.clone(), json_to_toml(value)?)))
                .collect::<Result<BTreeMap<_, _>, StoreError>>();
            Some(parameters.map(|parameters| {
                let previous = PreviousTrial {
                    index,
                    status,
                    parameters,
                    objective_value: trial.get("objective_value").and_then(JsonValue::as_f64),
                };
                (index, previous)
            }))
        })
        .collect()
}

fn previous_trial_observations(
    output: Option<&JsonValue>,
) -> Result<Vec<OptimizerObservation>, StoreError> {
    Ok(previous_trial_parameters(output)?
        .into_values()
        .filter_map(|trial| {
            let objective_value = trial.objective_value?;
            if !objective_value.is_finite() {
                return None;
            }
            Some(OptimizerObservation {
                parameters: trial.parameters,
                objective_value,
            })
        })
        .collect())
}

fn json_to_toml(value: &JsonValue) -> Result<toml::Value, StoreError> {
    match value {
        JsonValue::Null => Err(StoreError::store(
            "cannot convert null tuning parameter to TOML",
        )),
        JsonValue::Bool(value) => Ok(toml::Value::Boolean(*value)),
        JsonValue::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Ok(toml::Value::Integer(integer))
            } else if let Some(float) = value.as_f64() {
                Ok(toml::Value::Float(float))
            } else {
                Err(StoreError::store(
                    "unsupported JSON number in tuning parameter",
                ))
            }
        }
        JsonValue::String(value) => Ok(toml::Value::String(value.clone())),
        JsonValue::Array(values) => values
            .iter()
            .map(json_to_toml)
            .collect::<Result<Vec<_>, _>>()
            .map(toml::Value::Array),
        JsonValue::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_toml(value)?)))
            .collect::<Result<toml::map::Map<_, _>, StoreError>>()
            .map(toml::Value::Table),
    }
}

fn parameters_to_json(parameters: &BTreeMap<String, toml::Value>) -> Result<JsonValue, StoreError> {
    serde_json::to_value(parameters)
        .map_err(|err| StoreError::store(format!("failed to serialize tuning parameters: {err}")))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

pub fn placeholder_output(max_trials: usize) -> JsonValue {
    json!({
        "completed_trials": 0,
        "running_trials": 0,
        "failed_trials": 0,
        "total_trials": max_trials,
        "trials": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        HyperparameterTuningCategoricalDomain, HyperparameterTuningFloatDomain,
        HyperparameterTuningIntegerDomain, HyperparameterTuningOptimizerSpec,
        MeasurementMetricQuantity, MeasurementMetricSpec,
    };

    #[test]
    fn random_trial_parameters_are_deterministic_by_index() {
        let parameters = BTreeMap::from([
            (
                "x".to_string(),
                HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                    min: 0.0,
                    max: 1.0,
                }),
            ),
            (
                "n".to_string(),
                HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
                    min: 2,
                    max: 10,
                    step: Some(2),
                }),
            ),
        ]);

        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::RandomSearch,
            params: json!({ "max_trials": 8, "seed": 7 }),
        };

        let first = trial_parameters(&optimizer, &parameters, 3).expect("first");
        let second = trial_parameters(&optimizer, &parameters, 3).expect("second");
        let different = trial_parameters(&optimizer, &parameters, 4).expect("different");

        assert_eq!(first, second);
        assert_ne!(first, different);
        let Some(toml::Value::Integer(n)) = first.get("n") else {
            panic!("integer parameter missing");
        };
        assert_eq!((n - 2) % 2, 0);
    }

    #[test]
    fn grid_trial_parameters_enumerate_finite_domains() {
        let parameters = BTreeMap::from([
            (
                "bins".to_string(),
                HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
                    min: 8,
                    max: 16,
                    step: Some(4),
                }),
            ),
            (
                "mode".to_string(),
                HyperparameterTuningParameterDomain::Categorical(
                    crate::core::HyperparameterTuningCategoricalDomain {
                        source: crate::core::ParameterValueSourceSpec {
                            values: vec![
                                toml::Value::String("auto".to_string()),
                                toml::Value::String("none".to_string()),
                            ],
                            ..Default::default()
                        },
                    },
                ),
            ),
        ]);
        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::GridSearch,
            params: json!({}),
        };

        assert_eq!(optimizer_trial_count(&optimizer, &parameters).unwrap(), 6);
        let trials = (0..6)
            .map(|index| trial_parameters(&optimizer, &parameters, index).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            trials
                .iter()
                .map(|trial| (trial["bins"].clone(), trial["mode"].clone()))
                .collect::<Vec<_>>(),
            vec![
                (
                    toml::Value::Integer(8),
                    toml::Value::String("auto".to_string())
                ),
                (
                    toml::Value::Integer(8),
                    toml::Value::String("none".to_string())
                ),
                (
                    toml::Value::Integer(12),
                    toml::Value::String("auto".to_string())
                ),
                (
                    toml::Value::Integer(12),
                    toml::Value::String("none".to_string())
                ),
                (
                    toml::Value::Integer(16),
                    toml::Value::String("auto".to_string())
                ),
                (
                    toml::Value::Integer(16),
                    toml::Value::String("none".to_string())
                ),
            ]
        );
    }

    #[test]
    fn grid_search_rejects_float_domains() {
        let parameters = BTreeMap::from([(
            "x".to_string(),
            HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                min: 0.0,
                max: 1.0,
            }),
        )]);
        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::GridSearch,
            params: json!({}),
        };

        let err = optimizer_trial_count(&optimizer, &parameters).expect_err("float grid");
        assert!(err.to_string().contains("does not support float"));
    }

    #[test]
    fn egobox_encoder_round_trips_mixed_parameters() {
        let parameters = BTreeMap::from([
            (
                "center".to_string(),
                HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                    min: 0.0,
                    max: 1.0,
                }),
            ),
            (
                "bins".to_string(),
                HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
                    min: 8,
                    max: 32,
                    step: Some(8),
                }),
            ),
            (
                "mode".to_string(),
                HyperparameterTuningParameterDomain::Categorical(
                    HyperparameterTuningCategoricalDomain {
                        source: crate::core::ParameterValueSourceSpec {
                            values: vec![
                                toml::Value::String("auto".to_string()),
                                toml::Value::String("none".to_string()),
                            ],
                            ..Default::default()
                        },
                    },
                ),
            ),
        ]);
        let encoder = ParameterEncoder::new(&parameters).expect("encoder");
        let point = BTreeMap::from([
            ("center".to_string(), toml::Value::Float(0.5)),
            ("bins".to_string(), toml::Value::Integer(16)),
            ("mode".to_string(), toml::Value::String("none".to_string())),
        ]);

        let encoded = encoder.encode(&point).expect("encoded");
        let decoded = encoder.decode(&encoded).expect("decoded");

        assert_eq!(decoded, point);
    }

    #[test]
    fn egobox_initial_design_uses_bounded_random_candidates() {
        let parameters = BTreeMap::from([(
            "x".to_string(),
            HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                min: 0.0,
                max: 1.0,
            }),
        )]);
        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::Egobox,
            params: json!({
                "max_trials": 4,
                "seed": 2,
                "initial_design": 3,
                "parallel_candidates": 2
            }),
        };

        let plan = plan_optimizer_trials(
            &optimizer,
            &parameters,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &[],
            MeasurementMode::Minimize,
            2,
        )
        .expect("egobox initial plan");

        assert_eq!(plan.total_trials, 4);
        assert_eq!(plan.candidates.len(), 2);
        for candidate in plan.candidates {
            let value = candidate.parameters["x"].as_float().unwrap();
            assert!((0.0..1.0).contains(&value));
        }
    }

    #[test]
    fn egobox_proposes_from_completed_observations() {
        let parameters = BTreeMap::from([(
            "x".to_string(),
            HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                min: 0.0,
                max: 1.0,
            }),
        )]);
        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::Egobox,
            params: json!({
                "max_trials": 4,
                "seed": 2,
                "initial_design": 2,
                "parallel_candidates": 1,
                "infill": "ei"
            }),
        };
        let previous_trials = BTreeMap::from([
            (
                0,
                PreviousTrial {
                    index: 0,
                    status: "completed".to_string(),
                    parameters: BTreeMap::from([("x".to_string(), toml::Value::Float(0.0))]),
                    objective_value: Some(1.0),
                },
            ),
            (
                1,
                PreviousTrial {
                    index: 1,
                    status: "completed".to_string(),
                    parameters: BTreeMap::from([("x".to_string(), toml::Value::Float(1.0))]),
                    objective_value: Some(0.0),
                },
            ),
        ]);
        let observations = previous_trials
            .values()
            .map(|trial| OptimizerObservation {
                parameters: trial.parameters.clone(),
                objective_value: trial.objective_value.unwrap(),
            })
            .collect::<Vec<_>>();
        let existing = BTreeSet::from([0, 1]);

        let plan = plan_optimizer_trials(
            &optimizer,
            &parameters,
            &existing,
            &previous_trials,
            &observations,
            MeasurementMode::Minimize,
            1,
        )
        .expect("egobox observed plan");

        assert_eq!(plan.total_trials, 4);
        assert_eq!(plan.candidates.len(), 3);
        assert!(plan.candidates.iter().any(|candidate| candidate.index == 2));
        let suggested = plan
            .candidates
            .iter()
            .find(|candidate| candidate.index == 2)
            .unwrap()
            .parameters["x"]
            .as_float()
            .unwrap();
        assert!((0.0..=1.0).contains(&suggested));
    }

    #[test]
    fn egobox_reconstructs_previous_trials_from_controller_output() {
        let parameters = BTreeMap::from([(
            "x".to_string(),
            HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                min: 0.0,
                max: 1.0,
            }),
        )]);
        let optimizer = HyperparameterTuningOptimizerSpec {
            algorithm: HyperparameterTuningAlgorithm::Egobox,
            params: json!({
                "max_trials": 3,
                "seed": 4,
                "initial_design": 2,
                "parallel_candidates": 1
            }),
        };
        let previous_trials = previous_trial_parameters(Some(&json!({
            "trials": [
                {
                    "index": 0,
                    "status": "completed",
                    "parameters": { "x": 0.25 },
                    "objective_value": 1.0
                }
            ]
        })))
        .expect("previous trials");
        let observations = previous_trial_observations(Some(&json!({
            "trials": [
                {
                    "index": 0,
                    "status": "completed",
                    "parameters": { "x": 0.25 },
                    "objective_value": 1.0
                }
            ]
        })))
        .expect("previous observations");

        let plan = plan_optimizer_trials(
            &optimizer,
            &parameters,
            &BTreeSet::from([0]),
            &previous_trials,
            &observations,
            MeasurementMode::Minimize,
            1,
        )
        .expect("egobox restart plan");

        assert_eq!(plan.total_trials, 3);
        assert_eq!(plan.candidates[0].index, 0);
        assert_eq!(plan.candidates[0].parameters["x"].as_float(), Some(0.25));
        assert!(plan.candidates.iter().any(|candidate| candidate.index == 1));
    }

    #[test]
    fn egobox_random_fallback_avoids_known_encoded_points_when_possible() {
        let parameters = BTreeMap::from([(
            "x".to_string(),
            HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
                min: 1,
                max: 3,
                step: None,
            }),
        )]);
        let encoder = ParameterEncoder::new(&parameters).expect("encoder");
        let existing = vec![vec![1.0], vec![2.0]];
        let mut rng = Xoshiro256StarStar::seed_from_u64(1);

        let point = encoder
            .random_distinct_point(&mut rng, &existing)
            .expect("distinct point");

        assert_eq!(point["x"].as_integer(), Some(3));
    }

    #[test]
    fn objective_result_selects_requested_component() {
        let objective = HyperparameterTuningObjectiveSpec {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::Metric(MeasurementMetricQuantity {
                metric: AccumulatorMetricName::Mean,
                component: Some("imag".to_string()),
            }),
            metric: None,
            mode: MeasurementMode::Minimize,
        };
        let results = vec![
            crate::core::MeasurementResult {
                name: AccumulatorMetricName::Mean,
                component: Some("real".to_string()),
                value: 1.0,
                uncertainty: None,
                sample_count: 10,
                completed_samples_per_second: None,
            },
            crate::core::MeasurementResult {
                name: AccumulatorMetricName::Mean,
                component: Some("imag".to_string()),
                value: 2.0,
                uncertainty: None,
                sample_count: 10,
                completed_samples_per_second: None,
            },
        ];

        let selected = objective_result(&objective, &results).expect("selected result");
        assert_eq!(selected.value, 2.0);
    }

    #[test]
    fn objective_result_rejects_ambiguous_metric() {
        let objective = HyperparameterTuningObjectiveSpec {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue),
            metric: Some(MeasurementMetricSpec::Name(AccumulatorMetricName::Mean)),
            mode: MeasurementMode::Minimize,
        };
        let results = vec![
            crate::core::MeasurementResult {
                name: AccumulatorMetricName::Mean,
                component: Some("real".to_string()),
                value: 1.0,
                uncertainty: None,
                sample_count: 10,
                completed_samples_per_second: None,
            },
            crate::core::MeasurementResult {
                name: AccumulatorMetricName::Mean,
                component: Some("imag".to_string()),
                value: 2.0,
                uncertainty: None,
                sample_count: 10,
                completed_samples_per_second: None,
            },
        ];

        let err = objective_result(&objective, &results).expect_err("ambiguous objective");
        assert!(err.contains("multiple"));
    }

    #[test]
    fn objective_failure_reason_includes_context_and_available_results() {
        let objective = HyperparameterTuningObjectiveSpec {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::Metric(MeasurementMetricQuantity {
                metric: AccumulatorMetricName::Mean,
                component: Some("missing".to_string()),
            }),
            metric: None,
            mode: MeasurementMode::Minimize,
        };
        let results = vec![crate::core::MeasurementResult {
            name: AccumulatorMetricName::Mean,
            component: Some("real".to_string()),
            value: 1.0,
            uncertainty: None,
            sample_count: 10,
            completed_samples_per_second: None,
        }];

        let reason = objective_failure_reason(3, 42, &objective, &results, "not found");
        assert!(reason.contains("trial 3"));
        assert!(reason.contains("child_run_id=42"));
        assert!(reason.contains("source_task=sample"));
        assert!(reason.contains("Mean(component=missing)"));
        assert!(reason.contains("available_results=[Mean(component=real)]"));
    }
}
