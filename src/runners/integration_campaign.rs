use crate::api::runs::{ChildRunRequest, create_child_run};
use crate::core::{
    AggregationStore, ControlPlaneStore, ControllerChildOutput, ControllerChildState,
    ControllerTaskOutput, DerivedResultSnapshot, IntegrationCampaignAllocationAlgorithm,
    IntegrationCampaignAllocationSpec, IntegrationCampaignChildOutput, IntegrationCampaignOutput,
    IntegrationCampaignStopCondition, MeasurementResult, RunReadStore, RunSpecStore, RunTask,
    RunTaskSpec, RunTaskState, RunTaskStore, StoreError, StoreResultExt, TaskMeasurementOutput,
    WorkerRole,
};
use crate::runners::controller_child::{
    ControllerAssignmentPlan, apply_controller_assignment_plan, finish_controller,
    load_controller_children, load_published_child_result,
};
use crate::services::measurement::project_measurement_results;
use crate::services::results::combine_independent_observables;
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

        let mut by_label = load_controller_children(&self.store, self.run_id, &self.task)
            .await?
            .into_iter()
            .map(|(label, run)| (label, run.run_id))
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
                    run: child.run.clone(),
                    replacements: child.replacements.clone(),
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
        let mut previous_selected = previous_output
            .map(|output| output.selected_child_run_ids.clone())
            .unwrap_or_default();

        let mut states = Vec::with_capacity(children.len());
        let mut failure = None;
        for child in children {
            let run_id = *by_label
                .get(&child.name)
                .ok_or_else(|| StoreError::store("created campaign child is missing"))?;
            let (mut persisted, final_stage_ready, work_samples) =
                load_published_child_result(&self.store, run_id).await?;
            let completed_samples_per_second =
                self.latest_throughput(run_id).await?.or_else(|| {
                    previous_output
                        .and_then(|output| {
                            output
                                .children
                                .iter()
                                .find(|c| c.child.child_run_id == Some(run_id))
                        })
                        .and_then(|child| child.child.completed_samples_per_second)
                });
            let projected = persisted.accumulator.as_ref().and_then(|accumulator| {
                project_measurement_results(
                    accumulator,
                    measurement,
                    completed_samples_per_second,
                    &persisted.source_task,
                )
                .ok()
                .map(|results| TaskMeasurementOutput::Completed { results })
            });
            persisted.output = match persisted.output {
                failed @ Some(TaskMeasurementOutput::Failed { .. }) => failed,
                _ => projected,
            };
            let observed = persisted.controller_output();
            let child_failure_reason = observed
                .failure_reason
                .map(|reason| format!("campaign child '{}' failed: {reason}", child.name));
            if let Some(reason) = &child_failure_reason {
                failure.get_or_insert_with(|| reason.clone());
            }
            states.push(ChildState {
                final_stage_ready,
                work_samples,
                name: child.name.clone(),
                coefficient: child.coefficient,
                run_id,
                status: observed.status,
                task_state: persisted.task_state,
                result_source: persisted.source,
                accumulator: persisted.accumulator,
                completed_samples_per_second,
                measurement: observed.measurement,
                failure_reason: child_failure_reason,
            });
        }

        let workers = self.store.list_nodes(None).await?;
        // Prefer placements that still have a live sampler when capacity shrinks.
        // Placement may commit before its display snapshot during recovery.
        let saved_selected = previous_selected;
        previous_selected = workers
            .iter()
            .filter_map(|node| node.desired_assignment)
            .filter(|target| {
                target.role == WorkerRole::SamplerAggregator
                    && by_label.values().any(|id| *id == target.run_id)
            })
            .map(|target| target.run_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for id in saved_selected {
            if !previous_selected.contains(&id) {
                previous_selected.push(id);
            }
        }
        let roles = workers
            .iter()
            .filter_map(|node| node.desired_assignment)
            .filter(|a| a.run_id == self.run_id || by_label.values().any(|id| *id == a.run_id));
        let mut worker_counts = (0, 0);
        for assignment in roles {
            match assignment.role {
                WorkerRole::SamplerAggregator => worker_counts.0 += 1,
                WorkerRole::Evaluator => worker_counts.1 += 1,
            }
        }
        let capacity = allocation
            .max_active_runs
            .min(worker_counts.0)
            .min(worker_counts.1 / allocation.min_evaluators_per_run);
        let grants = self
            .store
            .allocate_campaign_samples(self.task.id, &[])
            .await?;
        let unallocated = stop_condition
            .max_total_samples
            .map(|cap| cap.saturating_sub(grants.values().sum::<i64>()));
        let outstanding = states.iter().any(|child| {
            child.task_state != RunTaskState::Completed
                && grants.get(&child.run_id).copied().unwrap_or(0) > child.work_samples
        });

        let combined_results = combine_measurements(&states);
        let combined_measurement = combined_results
            .clone()
            .map(|results| TaskMeasurementOutput::Completed { results });
        let result_snapshot_id = self
            .persist_derived_result(&states, combined_results.as_deref(), previous_output)
            .await?;
        let total_samples = states.iter().map(|child| child.work_samples).sum::<i64>();
        let pilots_complete = states.iter().all(|child| {
            child.final_stage_ready && child.sample_count() >= allocation.min_samples_per_child
        });
        let final_results_ready = states.iter().all(|child| child.final_stage_ready);

        if (combined_results.is_none() || !final_results_ready)
            && stop_condition
                .max_total_samples
                .is_some_and(|limit| total_samples >= limit)
        {
            failure.get_or_insert_with(|| {
                "campaign sample budget exhausted before every child published a usable result"
                    .into()
            });
        }
        if let Some(reason) = failure {
            let output = build_output(
                &states,
                &[],
                total_samples,
                worker_counts,
                combined_measurement,
                result_snapshot_id,
                allocation,
            );
            self.persist_output(&output).await?;
            return finish_controller(&self.store, self.run_id, self.task.id, Err(&reason)).await;
        }

        let target_reached = combined_results.as_deref().is_some_and(|results| {
            campaign_should_stop(results, total_samples, pilots_complete, stop_condition)
        });
        // Finish granted work before publishing a terminal estimate. After draining,
        // recheck the error target against all accepted samples before deciding to stop.
        let draining =
            outstanding && (target_reached || previous_output.is_some_and(|o| o.draining));
        if target_reached && !outstanding {
            let mut output = build_output(
                &states,
                &[],
                total_samples,
                worker_counts,
                combined_measurement.clone(),
                result_snapshot_id,
                allocation,
            );
            // Do not publish completion until the retiring workers have saved
            // their checkpoints and released their current placements.
            if workers
                .iter()
                .filter_map(|node| node.current_assignment)
                .any(|target| by_label.values().any(|id| *id == target.run_id))
            {
                output.draining = true;
                self.persist_output(&output).await?;
                apply_controller_assignment_plan(
                    &self.store,
                    ControllerAssignmentPlan::new(self.run_id, Vec::new()),
                )
                .await?;
                return Ok(false);
            }
            self.persist_output(&output).await?;
            if let Some(measurement) = &combined_measurement {
                self.store
                    .persist_task_measurement_output(self.task.id, measurement)
                    .await?;
            }
            return finish_controller(&self.store, self.run_id, self.task.id, Ok(())).await;
        }

        let can_allocate = unallocated != Some(0) && !draining;
        let eligible = states
            .iter()
            .filter(|child| child.task_state != RunTaskState::Completed)
            .filter(|child| {
                can_allocate || grants.get(&child.run_id).copied().unwrap_or(0) > child.work_samples
            })
            .map(|child| child.run_id)
            .collect::<BTreeSet<_>>();
        if eligible.is_empty() {
            let reason =
                "integration campaign exhausted all child runs before reaching its stop condition";
            let output = build_output(
                &states,
                &[],
                total_samples,
                worker_counts,
                combined_measurement,
                result_snapshot_id,
                allocation,
            );
            self.persist_output(&output).await?;
            return finish_controller(&self.store, self.run_id, self.task.id, Err(reason)).await;
        }

        let selected = schedule_children(
            &states,
            &grants,
            &previous_selected,
            &eligible,
            allocation,
            capacity,
        );
        let requests = if can_allocate {
            selected
                .iter()
                .filter_map(|id| {
                    let child = states.iter().find(|child| child.run_id == *id)?;
                    if grants.get(id).copied().unwrap_or(0) > child.work_samples {
                        return None;
                    }
                    let pilot_remaining = allocation.min_samples_per_child.max(2).saturating_sub(
                        if child.final_stage_ready {
                            child.sample_count()
                        } else {
                            0
                        },
                    );
                    let quantum = if child.needs_pilot(allocation.min_samples_per_child)
                        && pilot_remaining > 0
                    {
                        allocation.allocation_window_samples.min(pilot_remaining)
                    } else {
                        allocation.allocation_window_samples
                    };
                    Some((*id, child.work_samples.saturating_add(quantum)))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Grants commit before placement and are cumulative across stages. A crash
        // between the two steps resumes the same quantum, never grants it twice.
        if !requests.is_empty() {
            self.store
                .allocate_campaign_samples(self.task.id, &requests)
                .await?;
        }
        apply_controller_assignment_plan(
            &self.store,
            ControllerAssignmentPlan::new(self.run_id, selected.clone()),
        )
        .await?;
        let mut output = build_output(
            &states,
            &selected,
            total_samples,
            worker_counts,
            combined_measurement,
            result_snapshot_id,
            allocation,
        );
        output.draining = draining;
        self.persist_output(&output).await?;
        Ok(false)
    }

    async fn latest_throughput(&self, run_id: i32) -> Result<Option<f64>, StoreError> {
        let latest = self
            .store
            .get_sampler_performance_history(run_id, 1, None)
            .await?
            .into_iter()
            .map(|entry| entry.metrics.completed_samples_per_second)
            .find(|rate| rate.is_finite() && *rate > 0.0);
        Ok(latest)
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
    final_stage_ready: bool,
    work_samples: i64,
    name: String,
    coefficient: f64,
    run_id: i32,
    status: ControllerChildState,
    task_state: RunTaskState,
    result_source: crate::core::ResultSourceRef,
    accumulator: Option<crate::evaluation::AccumulatorState>,
    completed_samples_per_second: Option<f64>,
    measurement: Option<TaskMeasurementOutput>,
    failure_reason: Option<String>,
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

    fn needs_pilot(&self, minimum: i64) -> bool {
        !self.final_stage_ready
            || self.sample_count() < minimum
            || self
                .results()
                .is_none_or(|r| r.iter().any(|m| m.uncertainty.is_none()))
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

/// Retain unfinished quanta; reconsider only slots whose own work drained.
/// The durable grant also identifies an interrupted quantum after controller recovery.
fn schedule_children(
    states: &[ChildState],
    grants: &BTreeMap<i32, i64>,
    incumbents: &[i32],
    eligible: &BTreeSet<i32>,
    allocation: &IntegrationCampaignAllocationSpec,
    capacity: usize,
) -> Vec<i32> {
    let mut selected = incumbents
        .iter()
        .copied()
        .filter(|id| {
            eligible.contains(id)
                && states.iter().any(|child| {
                    child.run_id == *id && grants.get(id).copied().unwrap_or(0) > child.work_samples
                })
        })
        .take(capacity)
        .collect::<Vec<_>>();
    let mut candidates = states
        .iter()
        .filter(|child| eligible.contains(&child.run_id) && !selected.contains(&child.run_id))
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        let a_pilot = a.needs_pilot(allocation.min_samples_per_child);
        let b_pilot = b.needs_pilot(allocation.min_samples_per_child);
        let unfinished = |child: &ChildState| {
            grants.get(&child.run_id).copied().unwrap_or(0) > child.work_samples
        };
        unfinished(b)
            .cmp(&unfinished(a))
            .then_with(|| b_pilot.cmp(&a_pilot))
            .then_with(|| {
                if a_pilot {
                    a.work_samples.cmp(&b.work_samples)
                } else {
                    b.score(allocation.algorithm)
                        .unwrap_or(0.0)
                        .total_cmp(&a.score(allocation.algorithm).unwrap_or(0.0))
                }
            })
            .then_with(|| {
                incumbents
                    .contains(&b.run_id)
                    .cmp(&incumbents.contains(&a.run_id))
            })
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    selected.extend(
        candidates
            .into_iter()
            .take(capacity.saturating_sub(selected.len()))
            .map(|c| c.run_id),
    );
    selected
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
        let mut basic_mean = 0.0;
        let mut basic_variance = Some(0.0);
        let mut has_basic = false;
        for child in states {
            let result = child.results()?.iter().find(|result| {
                result.name == template.name && result.component == template.component
            })?;
            value += child.coefficient * result.value;
            let (mean, error) = match &result.basic {
                Some(basic) => {
                    has_basic = true;
                    (basic.mean, basic.error)
                }
                None => (result.value, result.uncertainty),
            };
            basic_mean += child.coefficient * mean;
            basic_variance = basic_variance
                .zip(error)
                .map(|(sum, e)| sum + child.coefficient.powi(2) * e.powi(2));
            variance += child.coefficient.powi(2) * result.uncertainty?.powi(2);
            sample_count = sample_count.saturating_add(result.sample_count);
        }
        combined.push(MeasurementResult {
            estimator: crate::core::EstimateKind::Combined,
            basic: (has_basic && basic_mean.is_finite()).then(|| crate::core::BasicEstimate {
                mean: basic_mean,
                error: basic_variance.filter(|v| v.is_finite()).map(f64::sqrt),
            }),
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
    worker_counts: (usize, usize),
    combined_measurement: Option<TaskMeasurementOutput>,
    result_snapshot_id: Option<String>,
    allocation: &IntegrationCampaignAllocationSpec,
) -> IntegrationCampaignOutput {
    let selected = selected.iter().copied().collect::<BTreeSet<_>>();
    IntegrationCampaignOutput {
        completed_children: states
            .iter()
            .filter(|child| child.task_state == RunTaskState::Completed)
            .count(),
        running_children: selected.len(),
        total_children: states.len(),
        total_samples,
        selected_child_run_ids: selected.iter().copied().collect(),
        max_active_runs: allocation.max_active_runs,
        sampler_workers: worker_counts.0,
        evaluator_workers: worker_counts.1,
        worker_capacity: allocation
            .max_active_runs
            .min(worker_counts.0)
            .min(worker_counts.1 / allocation.min_evaluators_per_run),
        draining: false,
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
                    failure_reason: child.failure_reason.clone(),
                },
                selected: selected.contains(&child.run_id),
                score: child
                    .score(allocation.algorithm)
                    .filter(|score| score.is_finite()),
                absolute_results: child.accumulator.as_ref().map(|accumulator| {
                    crate::services::measurement::absolute_component_results(
                        accumulator,
                        child.results().unwrap_or_default(),
                    )
                }),
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
            final_stage_ready: true,
            work_samples: samples,
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
                    estimator: crate::core::EstimateKind::Pooled,
                    basic: None,
                    name: AccumulatorMetricName::Mean,
                    component: None,
                    value,
                    uncertainty: Some(error),
                    sample_count: samples,
                }],
            }),
            failure_reason: None,
        }
    }

    fn complex_child(
        run_id: i32,
        coefficient: f64,
        real: (f64, f64),
        imag: (f64, f64),
        samples: i64,
    ) -> ChildState {
        let mut child = child(run_id, coefficient, real.0, real.1, samples);
        child.measurement = Some(TaskMeasurementOutput::Completed {
            results: vec![
                MeasurementResult {
                    estimator: crate::core::EstimateKind::Pooled,
                    basic: None,
                    name: AccumulatorMetricName::Mean,
                    component: Some("real".to_string()),
                    value: real.0,
                    uncertainty: Some(real.1),
                    sample_count: samples,
                },
                MeasurementResult {
                    estimator: crate::core::EstimateKind::Pooled,
                    basic: None,
                    name: AccumulatorMetricName::Mean,
                    component: Some("imag".to_string()),
                    value: imag.0,
                    uncertainty: Some(imag.1),
                    sample_count: samples,
                },
            ],
        });
        child
    }

    #[test]
    fn quanta_switch_independently_and_ties_keep_the_incumbent() {
        let allocation = IntegrationCampaignAllocationSpec {
            min_samples_per_child: 0,
            algorithm: IntegrationCampaignAllocationAlgorithm::LargestVariance,
            ..Default::default()
        };
        let mut states = vec![
            child(1, 1., 1., 1., 100),
            child(2, 1., 1., 0.1, 20),
            child(3, 1., 1., 2., 100),
        ];
        let grants = [(1, 100), (2, 100), (3, 100)].into();
        let eligible = [1, 2, 3].into();
        assert_eq!(
            schedule_children(&states, &grants, &[1, 2], &eligible, &allocation, 2),
            vec![2, 3],
            "B retains its unfinished quantum while C replaces A"
        );
        states[2] = child(3, 1., 1., 1., 100);
        assert_eq!(
            schedule_children(&states, &grants, &[1, 2], &eligible, &allocation, 2),
            vec![2, 1],
            "equal score does not restart A"
        );
        assert!(schedule_children(&states, &grants, &[1, 2], &eligible, &allocation, 0).is_empty());
    }

    #[test]
    fn committed_quantum_survives_missing_controller_output() {
        let allocation = IntegrationCampaignAllocationSpec {
            min_samples_per_child: 0,
            ..Default::default()
        };
        let states = vec![child(1, 1., 1., 0.1, 20), child(2, 1., 1., 10., 100)];
        assert_eq!(
            schedule_children(
                &states,
                &[(1, 100), (2, 100)].into(),
                &[],
                &[1, 2].into(),
                &allocation,
                1
            ),
            vec![1]
        );
    }

    #[test]
    fn unavailable_uncertainty_keeps_exploration_priority_after_minimum_samples() {
        let mut uncertain = child(1, 1., 1., 0., 100);
        if let Some(TaskMeasurementOutput::Completed { results }) = &mut uncertain.measurement {
            results[0].uncertainty = None;
        }
        assert_eq!(
            select_children(
                &[uncertain, child(2, 1., 1., 10., 100)],
                IntegrationCampaignAllocationAlgorithm::LargestVariance,
                32,
                1
            ),
            vec![1]
        );
    }

    #[test]
    fn combines_weighted_independent_measurements() {
        let results =
            combine_measurements(&[child(1, 2.0, 3.0, 0.4, 10), child(2, -1.0, 1.0, 0.3, 20)])
                .expect("combined");
        assert_eq!(results[0].value, 5.0);
        assert!((results[0].uncertainty.unwrap() - 0.73_f64.sqrt()).abs() < 1e-12);
        assert_eq!(results[0].sample_count, 30);
        assert_eq!(results[0].estimator, crate::core::EstimateKind::Combined);
        let mut children = [child(1, 2.0, 3.0, 0.4, 10), child(2, -1.0, 1.0, 0.3, 20)];
        if let Some(TaskMeasurementOutput::Completed { results }) = &mut children[0].measurement {
            results[0].estimator = crate::core::EstimateKind::Stratified;
            results[0].basic = Some(crate::core::BasicEstimate {
                mean: 10.0,
                error: Some(0.1),
            });
        }
        let compared = combine_measurements(&children).unwrap();
        assert_eq!(compared[0].value, results[0].value);
        assert_eq!(compared[0].uncertainty, results[0].uncertainty);
        assert_eq!(compared[0].basic.as_ref().unwrap().mean, 19.0);
        assert!(
            (compared[0].basic.as_ref().unwrap().error.unwrap() - 0.13_f64.sqrt()).abs() < 1e-12
        );
    }

    #[test]
    fn combines_weighted_complex_measurements_component_wise() {
        let results = combine_measurements(&[
            complex_child(1, 2.0, (3.0, 0.4), (-2.0, 0.2), 10),
            complex_child(2, -0.5, (1.0, 0.3), (4.0, 0.6), 20),
        ])
        .expect("combined");

        let real = &results[0];
        assert_eq!(real.component.as_deref(), Some("real"));
        assert_eq!(real.value, 5.5);
        assert!((real.uncertainty.unwrap() - 0.6625_f64.sqrt()).abs() < 1e-12);
        assert_eq!(real.sample_count, 30);

        let imag = &results[1];
        assert_eq!(imag.component.as_deref(), Some("imag"));
        assert_eq!(imag.value, -6.0);
        assert!((imag.uncertainty.unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(imag.sample_count, 30);
    }

    fn select_children(
        states: &[ChildState],
        algorithm: IntegrationCampaignAllocationAlgorithm,
        minimum: i64,
        limit: usize,
    ) -> Vec<i32> {
        schedule_children(
            states,
            &BTreeMap::new(),
            &[],
            &states
                .iter()
                .filter(|c| c.task_state != RunTaskState::Completed)
                .map(|c| c.run_id)
                .collect(),
            &IntegrationCampaignAllocationSpec {
                algorithm,
                min_samples_per_child: minimum,
                ..Default::default()
            },
            limit,
        )
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
    fn variance_reduction_rate_does_not_balance_samples_after_pilot() {
        let selected = select_children(
            &[child(1, 1.0, 1.0, 0.5, 120), child(2, 1.0, 1.0, 0.1, 100)],
            IntegrationCampaignAllocationAlgorithm::VarianceReductionRate,
            50,
            1,
        );
        assert_eq!(selected, vec![1]);
    }

    #[test]
    fn insufficient_stratum_coverage_cannot_converge_a_campaign() {
        use crate::core::{
            AccumulatorConfig, AccumulatorMetricSelector, StratificationConfig, StratificationPlan,
        };
        use crate::evaluation::accumulator::StratifiedAccumulatorState;
        let plan = StratificationPlan {
            proposal_id: "campaign".into(),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![0.], vec![1.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
                scales: None,
            },
            probabilities: Some(vec![0.5, 0.5]),
            components: vec!["value".into()],
        };
        let mut state = StratifiedAccumulatorState::new(plan, AccumulatorConfig::scalar()).unwrap();
        for _ in 0..64 {
            state.push(0, &[0.], 2., false).unwrap();
            state.push(1, &[1.], 2., true).unwrap();
        }
        let selector = AccumulatorMetricSelector {
            name: AccumulatorMetricName::Mean,
            component: Some("value".into()),
        };
        let measurement = state.metric(&selector).unwrap();
        assert_eq!(measurement.value, 1.0);
        assert_eq!(measurement.uncertainty, None);
        let stop = IntegrationCampaignStopCondition {
            min_total_samples: 0,
            max_total_samples: None,
            absolute_error: Some(1.0),
            relative_error: None,
        };
        assert!(!campaign_should_stop(
            std::slice::from_ref(&measurement),
            128,
            true,
            &stop
        ));
        let mut child = child(1, 1.0, 1.0, 0.0, 128);
        child.measurement = Some(TaskMeasurementOutput::Completed {
            results: vec![measurement],
        });
        assert!(
            child
                .score(IntegrationCampaignAllocationAlgorithm::LargestVariance)
                .is_none()
        );
        assert!(combine_measurements(&[child]).is_none());
        for _ in 0..2 {
            state.push(0, &[0.], 2., true).unwrap();
        }
        let measurement = state.metric(&selector).unwrap();
        assert_eq!(measurement.uncertainty, Some(0.0));
        assert!(campaign_should_stop(&[measurement], 130, true, &stop));
    }

    #[test]
    fn error_target_waits_for_every_child_pilot() {
        let results = [MeasurementResult {
            estimator: crate::core::EstimateKind::Pooled,
            basic: None,
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
