use crate::core::{
    AccumulatorMetricName, AggregationStore, ControlPlaneStore, HyperparameterTuningObjectiveSpec,
    HyperparameterTuningParameterDomain, MeasurementMode, MeasurementQuantityName,
    MeasurementQuantitySpec, RunReadStore, RunSpecStore, RunTask, RunTaskSpec, RunTaskState,
    RunTaskStore, StoreError, TaskMeasurementOutput,
};
use crate::runners::controller_child::{
    ControllerChildRunRequest, choose_child_capacity, create_controller_child_run,
    list_child_runs_for_task, load_child_task_measurement,
    redistribute_parent_assignments_to_children,
};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

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
        self.store
            .clear_desired_assignments_for_run(self.run_id)
            .await?;

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

        let total_trials = optimizer.max_trials;
        let mut trials = Vec::with_capacity(total_trials);
        let mut completed_count = 0usize;
        let mut running_count = 0usize;
        let mut failed_count = 0usize;

        for index in 0..total_trials {
            let label = index.to_string();
            let values = trial_parameters(parameters, optimizer.seed.unwrap_or(0), index)?;
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
                            trials.push(HyperparameterTrialOutput {
                                index,
                                parameters: parameters_json,
                                child_run_id: Some(child.run_id),
                                status: "failed".to_string(),
                                objective_value: None,
                                objective_uncertainty: None,
                                measurement: Some(TaskMeasurementOutput::Completed { results }),
                                failure_reason: Some(reason),
                            });
                        }
                    }
                }
                Some(TaskMeasurementOutput::Failed { reason }) => {
                    failed_count += 1;
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
                        failure_reason: Some(reason),
                    });
                }
                None => {
                    if measurement_output.task_state == RunTaskState::Completed {
                        failed_count += 1;
                        trials.push(HyperparameterTrialOutput {
                            index,
                            parameters: parameters_json,
                            child_run_id: Some(child.run_id),
                            status: "failed".to_string(),
                            objective_value: None,
                            objective_uncertainty: None,
                            measurement: None,
                            failure_reason: Some(format!(
                                "child source task '{}' completed without measurement output",
                                objective.source_task
                            )),
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
            self.persist_output(completed_count, running_count, failed_count, trials)
                .await?;
            self.store
                .fail_run_task(
                    self.task.id,
                    &format!("hyperparameter tuning failed: failed_trials={failed_count}"),
                )
                .await?;
            return Ok(true);
        }

        if completed_count + failed_count == total_trials {
            self.persist_output(completed_count, running_count, failed_count, trials)
                .await?;
            self.store
                .update_run_task_progress(self.task.id, total_trials as i64, completed_count as i64)
                .await?;
            self.store.complete_run_task(self.task.id).await?;
            return Ok(true);
        }

        let mut created_child_run_ids = Vec::new();
        let mut capacity = choose_child_capacity(*max_concurrent_trials, running_count);
        if capacity > 0 {
            for index in 0..total_trials {
                if capacity == 0 {
                    break;
                }
                let label = index.to_string();
                if child_runs_by_label.contains_key(label.as_str()) {
                    continue;
                }
                let replacements =
                    trial_parameters(parameters, optimizer.seed.unwrap_or(0), index)?;
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
                        self.persist_output(completed_count, running_count, failed_count, trials)
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
        self.persist_output(completed_count, running_count, failed_count, trials)
            .await?;
        Ok(false)
    }

    async fn persist_output(
        &self,
        completed_trials: usize,
        running_trials: usize,
        failed_trials: usize,
        trials: Vec<HyperparameterTrialOutput>,
    ) -> Result<(), StoreError> {
        let (best_trial, best_objective_value) =
            best_trial(&trials, objective_mode(&self.task.task));
        let output = serde_json::to_value(HyperparameterTuningOutput {
            total_trials: trials.len(),
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

fn trial_parameters(
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
    seed: u64,
    index: usize,
) -> Result<BTreeMap<String, toml::Value>, StoreError> {
    let mut rng = Xoshiro256StarStar::seed_from_u64(seed ^ splitmix64(index as u64));
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
            let index = rng.random_range(0..domain.values.len());
            Ok(domain.values[index].clone())
        }
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
        HyperparameterTuningFloatDomain, HyperparameterTuningIntegerDomain,
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

        let first = trial_parameters(&parameters, 7, 3).expect("first");
        let second = trial_parameters(&parameters, 7, 3).expect("second");
        let different = trial_parameters(&parameters, 7, 4).expect("different");

        assert_eq!(first, second);
        assert_ne!(first, different);
        let Some(toml::Value::Integer(n)) = first.get("n") else {
            panic!("integer parameter missing");
        };
        assert_eq!((n - 2) % 2, 0);
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
}
