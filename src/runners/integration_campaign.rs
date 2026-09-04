use crate::api::measurement::project_measurement_results;
use crate::api::results::combine_independent_observables;
use crate::api::runs::{ChildRunRequest, create_child_run};
use crate::core::{
    AggregationStore, ControlPlaneStore, ControllerChildOutput, ControllerChildState,
    ControllerTaskOutput, DerivedResultSnapshot, IntegrationCampaignAllocationAlgorithm,
    IntegrationCampaignChildOutput, IntegrationCampaignOutput, IntegrationCampaignStopCondition,
    MeasurementResult, RunReadStore, RunSpecStore, RunTask, RunTaskSpec, RunTaskState,
    RunTaskStore, SamplerRuntimeMetrics, StoreError, StoreResultExt, TaskMeasurementOutput,
};
use crate::runners::controller_child::{
    ControllerAssignmentPlan, apply_controller_assignment_plan, load_child_task_result,
};
use std::collections::{BTreeMap, BTreeSet};

const SPAWN_KIND: &str = "integration_campaign";

pub struct IntegrationCampaignRunner<S> {
    store: S,
    run_id: i32,
    task: RunTask,
}

impl<S> IntegrationCampaignRunner<S> {
    pub fn new(store: S, run_id: i32, task: RunTask) -> Self {
        Self {
            store,
            run_id,
            task,
        }
    }
}

impl<S> IntegrationCampaignRunner<S>
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
        let RunTaskSpec::IntegrationCampaign {
            children,
            measurement,
            stop_condition,
            allocation,
        } = &self.task.task
        else {
            return Err(StoreError::store(
                "integration campaign runner got another task kind",
            ));
        };

        let existing = self
            .store
            .get_child_runs_for_task(self.run_id, self.task.id, SPAWN_KIND)
            .await?;
        let mut by_label = existing
            .iter()
            .filter_map(|run| {
                run.spawn_label
                    .as_deref()
                    .map(|label| (label.to_string(), run.run_id))
            })
            .collect::<BTreeMap<_, _>>();
        for child in children {
            if by_label.contains_key(&child.name) {
                continue;
            }
            let created = create_child_run(
                &self.store,
                ChildRunRequest {
                    parent_run_id: self.run_id,
                    parent_task_id: Some(self.task.id),
                    spawn_kind: SPAWN_KIND.to_string(),
                    spawn_label: Some(child.name.clone()),
                    run_toml: child.run_toml.clone(),
                    replacements: BTreeMap::new(),
                },
            )
            .await
            .store_err()?;
            by_label.insert(child.name.clone(), created.run_id);
        }

        let previous_output = self
            .task
            .controller_output
            .as_ref()
            .and_then(ControllerTaskOutput::integration_campaign);
        let previous_selected = previous_output
            .map(|output| output.selected_child_run_ids.clone())
            .unwrap_or_default();
        let previous_allocation_start = previous_output
            .map(|output| output.allocation_started_total_samples)
            .unwrap_or(0);

        let mut states = Vec::with_capacity(children.len());
        let mut failure = None;
        for child in children {
            let run_id = *by_label
                .get(&child.name)
                .ok_or_else(|| StoreError::store("created campaign child is missing"))?;
            let persisted =
                load_child_task_result(&self.store, run_id, &measurement.source_task).await?;
            let completed_samples_per_second = self.latest_throughput(run_id).await?;
            let projected = persisted.accumulator.as_ref().and_then(|accumulator| {
                project_measurement_results(
                    accumulator,
                    &measurement.task_measurement(),
                    completed_samples_per_second,
                    &persisted.source_task,
                )
                .ok()
                .map(|results| TaskMeasurementOutput::Completed { results })
            });
            let output = match persisted.output {
                failed @ Some(TaskMeasurementOutput::Failed { .. }) => failed,
                _ if persisted.task_state == RunTaskState::Active => projected,
                output => output.or(projected),
            };
            if let Some(TaskMeasurementOutput::Failed { reason }) = &output {
                failure.get_or_insert_with(|| {
                    format!(
                        "campaign child '{}' measurement failed: {reason}",
                        child.name
                    )
                });
            }
            let status = match (&output, persisted.task_state) {
                (Some(TaskMeasurementOutput::Failed { .. }), _) => ControllerChildState::Failed,
                (_, state) => state.into(),
            };
            states.push(ChildState {
                name: child.name.clone(),
                coefficient: child.coefficient,
                run_id,
                status,
                task_state: persisted.task_state,
                result_source: persisted.source,
                accumulator: persisted.accumulator,
                completed_samples_per_second,
                measurement: output,
            });
        }

        let combined_results = combine_measurements(&states);
        let combined_measurement = combined_results
            .clone()
            .map(|results| TaskMeasurementOutput::Completed { results });
        let result_snapshot_id = self
            .persist_derived_result(&states, combined_results.as_deref(), previous_output)
            .await?;
        let total_samples = states.iter().map(ChildState::sample_count).sum::<i64>();
        let pilots_complete = states
            .iter()
            .all(|child| child.sample_count() >= allocation.min_samples_per_child);

        if let Some(reason) = failure {
            let output = build_output(
                &states,
                &[],
                total_samples,
                total_samples,
                combined_measurement,
                result_snapshot_id,
                allocation.algorithm,
            );
            self.persist_output(&output).await?;
            apply_controller_assignment_plan(
                &self.store,
                ControllerAssignmentPlan::replacing(
                    self.run_id,
                    states.iter().map(|child| child.run_id).collect(),
                    Vec::new(),
                ),
            )
            .await?;
            self.store.fail_run_task(self.task.id, &reason).await?;
            return Ok(true);
        }

        if combined_results.as_deref().is_some_and(|results| {
            campaign_should_stop(results, total_samples, pilots_complete, stop_condition)
        }) {
            let output = build_output(
                &states,
                &[],
                total_samples,
                total_samples,
                combined_measurement.clone(),
                result_snapshot_id,
                allocation.algorithm,
            );
            self.persist_output(&output).await?;
            if let Some(measurement) = &combined_measurement {
                self.store
                    .persist_task_measurement_output(self.task.id, measurement)
                    .await?;
            }
            apply_controller_assignment_plan(
                &self.store,
                ControllerAssignmentPlan::replacing(
                    self.run_id,
                    states.iter().map(|child| child.run_id).collect(),
                    Vec::new(),
                ),
            )
            .await?;
            self.store.complete_run_task(self.task.id).await?;
            return Ok(true);
        }

        let eligible = states
            .iter()
            .filter(|child| child.task_state != RunTaskState::Completed)
            .map(|child| child.run_id)
            .collect::<BTreeSet<_>>();
        if eligible.is_empty() {
            let reason =
                "integration campaign exhausted all child runs before reaching its stop condition";
            let output = build_output(
                &states,
                &[],
                total_samples,
                total_samples,
                combined_measurement,
                result_snapshot_id,
                allocation.algorithm,
            );
            self.persist_output(&output).await?;
            self.store.fail_run_task(self.task.id, reason).await?;
            return Ok(true);
        }

        let keep_window = total_samples.saturating_sub(previous_allocation_start)
            < allocation.allocation_window_samples;
        let retained = previous_selected
            .iter()
            .copied()
            .filter(|run_id| eligible.contains(run_id))
            .collect::<Vec<_>>();
        let (selected, allocation_started_total_samples) = if keep_window && !retained.is_empty() {
            (retained, previous_allocation_start)
        } else {
            (
                select_children(
                    &states,
                    allocation.algorithm,
                    allocation.min_samples_per_child,
                    allocation.max_active_runs,
                ),
                total_samples,
            )
        };

        apply_controller_assignment_plan(
            &self.store,
            ControllerAssignmentPlan::replacing(
                self.run_id,
                states.iter().map(|child| child.run_id).collect(),
                selected.clone(),
            ),
        )
        .await?;
        let output = build_output(
            &states,
            &selected,
            total_samples,
            allocation_started_total_samples,
            combined_measurement,
            result_snapshot_id,
            allocation.algorithm,
        );
        self.persist_output(&output).await?;
        Ok(false)
    }

    async fn latest_throughput(&self, run_id: i32) -> Result<Option<f64>, StoreError> {
        let latest = self
            .store
            .get_sampler_performance_history(run_id, 1, None)
            .await?
            .into_iter()
            .next();
        Ok(latest
            .and_then(|entry| {
                serde_json::from_value::<SamplerRuntimeMetrics>(entry.runtime_metrics).ok()
            })
            .map(|runtime| runtime.completed_samples_per_second)
            .filter(|rate| rate.is_finite() && *rate > 0.0))
    }

    async fn persist_output(&self, output: &IntegrationCampaignOutput) -> Result<(), StoreError> {
        self.store
            .persist_task_controller_output(
                self.task.id,
                &ControllerTaskOutput::IntegrationCampaign(output.clone()),
            )
            .await
    }

    async fn persist_derived_result(
        &self,
        states: &[ChildState],
        metrics: Option<&[MeasurementResult]>,
        previous: Option<&IntegrationCampaignOutput>,
    ) -> Result<Option<String>, StoreError> {
        let previous_id = previous.and_then(|output| output.result_snapshot_id.clone());
        let Some(metrics) = metrics else {
            return Ok(previous_id);
        };
        let sources = states
            .iter()
            .map(|state| state.result_source.clone())
            .collect::<Vec<_>>();
        let unchanged =
            previous.is_some_and(|output| {
                output.children.len() == states.len()
                    && output.children.iter().zip(states).all(|(old, new)| {
                        old.child.result_source.as_ref() == Some(&new.result_source)
                    })
                    && matches!(
                        output.combined_measurement.as_ref(),
                        Some(TaskMeasurementOutput::Completed { results }) if results == metrics
                    )
            });
        if unchanged {
            return Ok(previous_id);
        }
        let Some(observable_inputs) = states
            .iter()
            .map(|state| Some((state.coefficient, state.accumulator.as_ref()?)))
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(previous_id);
        };
        if observable_inputs
            .iter()
            .any(|(_, accumulator)| accumulator.sample_count() <= 0)
        {
            return Ok(previous_id);
        }
        let result = DerivedResultSnapshot {
            sources,
            metrics: metrics.to_vec(),
            observables: combine_independent_observables(&observable_inputs),
        };
        let payload = serde_json::to_value(result).map_err(|err| {
            StoreError::store(format!("failed to serialize campaign result: {err}"))
        })?;
        let id = self
            .store
            .persist_task_result_snapshot(self.run_id, self.task.id, &payload)
            .await?;
        Ok(Some(id.to_string()))
    }
}

#[derive(Debug, Clone)]
struct ChildState {
    name: String,
    coefficient: f64,
    run_id: i32,
    status: ControllerChildState,
    task_state: RunTaskState,
    result_source: crate::core::ResultSourceRef,
    accumulator: Option<crate::evaluation::AccumulatorState>,
    completed_samples_per_second: Option<f64>,
    measurement: Option<TaskMeasurementOutput>,
}

impl ChildState {
    fn results(&self) -> Option<&[MeasurementResult]> {
        match self.measurement.as_ref()? {
            TaskMeasurementOutput::Completed { results } => Some(results),
            TaskMeasurementOutput::Failed { .. } => None,
        }
    }

    fn sample_count(&self) -> i64 {
        self.results()
            .and_then(|results| results.iter().map(|result| result.sample_count).max())
            .unwrap_or(0)
    }

    fn score(&self, algorithm: IntegrationCampaignAllocationAlgorithm) -> Option<f64> {
        let results = self.results()?;
        let variance = results.iter().try_fold(0.0, |sum, result| {
            let uncertainty = result.uncertainty?;
            Some(sum + self.coefficient.powi(2) * uncertainty.powi(2))
        })?;
        match algorithm {
            IntegrationCampaignAllocationAlgorithm::LargestVariance => Some(variance),
            IntegrationCampaignAllocationAlgorithm::VarianceReductionRate => {
                let samples = self.sample_count().max(1) as f64;
                let throughput = self.completed_samples_per_second?;
                Some(variance * throughput / samples)
            }
        }
    }
}

fn select_children(
    states: &[ChildState],
    algorithm: IntegrationCampaignAllocationAlgorithm,
    min_samples_per_child: i64,
    limit: usize,
) -> Vec<i32> {
    let mut ranked = states
        .iter()
        .filter(|child| child.task_state != RunTaskState::Completed)
        .map(|child| {
            let pilot = child.sample_count() < min_samples_per_child;
            let score = child.score(algorithm).unwrap_or(0.0);
            (child.run_id, pilot, child.sample_count(), score)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| right.3.total_cmp(&left.3))
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(run_id, _, _, _)| run_id)
        .collect()
}

fn combine_measurements(states: &[ChildState]) -> Option<Vec<MeasurementResult>> {
    if states.iter().any(|child| child.results().is_none()) {
        return None;
    }
    let first = states.first()?.results()?;
    let mut combined = Vec::with_capacity(first.len());
    for template in first {
        let mut value = 0.0;
        let mut variance = 0.0;
        let mut sample_count = 0i64;
        for child in states {
            let result = child.results()?.iter().find(|result| {
                result.name == template.name && result.component == template.component
            })?;
            value += child.coefficient * result.value;
            variance += child.coefficient.powi(2) * result.uncertainty?.powi(2);
            sample_count = sample_count.saturating_add(result.sample_count);
        }
        combined.push(MeasurementResult {
            name: template.name,
            component: template.component.clone(),
            value,
            uncertainty: Some(variance.sqrt()),
            sample_count,
        });
    }
    Some(combined)
}

fn campaign_should_stop(
    results: &[MeasurementResult],
    total_samples: i64,
    pilots_complete: bool,
    stop: &IntegrationCampaignStopCondition,
) -> bool {
    if total_samples < stop.min_total_samples {
        return false;
    }
    if stop
        .max_total_samples
        .is_some_and(|maximum| total_samples >= maximum)
    {
        return true;
    }
    if !pilots_complete {
        return false;
    }
    let error_targets_met = results.iter().all(|result| {
        let Some(error) = result.uncertainty else {
            return false;
        };
        let absolute_met = stop.absolute_error.is_some_and(|target| error <= target);
        let relative_met = stop
            .relative_error
            .is_some_and(|target| result.value != 0.0 && error / result.value.abs() <= target);
        absolute_met || relative_met
    });
    (stop.absolute_error.is_some() || stop.relative_error.is_some()) && error_targets_met
}

fn build_output(
    states: &[ChildState],
    selected: &[i32],
    total_samples: i64,
    allocation_started_total_samples: i64,
    combined_measurement: Option<TaskMeasurementOutput>,
    result_snapshot_id: Option<String>,
    algorithm: IntegrationCampaignAllocationAlgorithm,
) -> IntegrationCampaignOutput {
    let selected = selected.iter().copied().collect::<BTreeSet<_>>();
    IntegrationCampaignOutput {
        completed_children: states
            .iter()
            .filter(|child| child.task_state == RunTaskState::Completed)
            .count(),
        running_children: states
            .iter()
            .filter(|child| child.task_state == RunTaskState::Active)
            .count(),
        total_children: states.len(),
        total_samples,
        selected_child_run_ids: selected.iter().copied().collect(),
        allocation_started_total_samples,
        combined_measurement,
        result_snapshot_id,
        children: states
            .iter()
            .map(|child| IntegrationCampaignChildOutput {
                name: child.name.clone(),
                coefficient: child.coefficient,
                child: ControllerChildOutput {
                    child_run_id: Some(child.run_id),
                    status: child.status,
                    result_source: Some(child.result_source.clone()),
                    completed_samples_per_second: child.completed_samples_per_second,
                    measurement: child.measurement.clone(),
                    failure_reason: match child.measurement.as_ref() {
                        Some(TaskMeasurementOutput::Failed { reason }) => Some(reason.clone()),
                        _ => None,
                    },
                },
                selected: selected.contains(&child.run_id),
                score: child.score(algorithm).filter(|score| score.is_finite()),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AccumulatorMetricName;

    fn child(run_id: i32, coefficient: f64, value: f64, error: f64, samples: i64) -> ChildState {
        ChildState {
            name: format!("graph-{run_id}"),
            coefficient,
            run_id,
            status: ControllerChildState::Active,
            task_state: RunTaskState::Active,
            result_source: crate::core::ResultSourceRef {
                run_id,
                task_id: i64::from(run_id),
                snapshot_id: None,
                sample_count: samples,
            },
            accumulator: None,
            completed_samples_per_second: Some(10.0),
            measurement: Some(TaskMeasurementOutput::Completed {
                results: vec![MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: None,
                    value,
                    uncertainty: Some(error),
                    sample_count: samples,
                }],
            }),
        }
    }

    #[test]
    fn combines_weighted_independent_measurements() {
        let results =
            combine_measurements(&[child(1, 2.0, 3.0, 0.4, 10), child(2, -1.0, 1.0, 0.3, 20)])
                .expect("combined");
        assert_eq!(results[0].value, 5.0);
        assert!((results[0].uncertainty.unwrap() - 0.73_f64.sqrt()).abs() < 1e-12);
        assert_eq!(results[0].sample_count, 30);
    }

    #[test]
    fn pilot_allocation_visits_least_sampled_child_first() {
        let selected = select_children(
            &[child(1, 1.0, 1.0, 0.1, 100), child(2, 1.0, 1.0, 1.0, 10)],
            IntegrationCampaignAllocationAlgorithm::LargestVariance,
            50,
            1,
        );
        assert_eq!(selected, vec![2]);
    }

    #[test]
    fn variance_allocation_selects_largest_weighted_uncertainty() {
        let selected = select_children(
            &[child(1, 1.0, 1.0, 0.5, 100), child(2, 3.0, 1.0, 0.2, 100)],
            IntegrationCampaignAllocationAlgorithm::LargestVariance,
            0,
            1,
        );
        assert_eq!(selected, vec![2]);
    }

    #[test]
    fn error_target_waits_for_every_child_pilot() {
        let results = [MeasurementResult {
            name: AccumulatorMetricName::Mean,
            component: None,
            value: 1.5,
            uncertainty: Some(0.0),
            sample_count: 2_012,
        }];
        let stop = IntegrationCampaignStopCondition {
            min_total_samples: 1_500,
            max_total_samples: Some(4_000),
            absolute_error: Some(1e-12),
            relative_error: None,
        };

        assert!(!campaign_should_stop(&results, 2_012, false, &stop));
        assert!(campaign_should_stop(&results, 2_012, true, &stop));
        assert!(campaign_should_stop(&results, 4_000, false, &stop));
    }

    #[cfg(feature = "gammaloop")]
    #[test]
    fn gammaloop_accumulator_exposes_the_result_bundle_path() {
        let value = crate::evaluation::AccumulatorState::empty_gammaloop()
            .to_json()
            .expect("json");
        assert!(value.pointer("/bundle/histograms").is_some());
    }
}
