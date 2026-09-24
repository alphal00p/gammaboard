//! Interval summaries use the same clipped counter differences as the graphs.
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Mean {
    total: f64,
    weight: f64,
    invalid: bool,
}

impl Mean {
    fn add(&mut self, value: Option<f64>, weight: f64) {
        if let Some(value) = value {
            self.total += value * weight;
            self.weight += weight;
        } else {
            self.invalid = true;
        }
    }

    pub(super) fn value(&self) -> Option<f64> {
        (!self.invalid && self.weight > 0.0).then(|| self.total / self.weight)
    }
}

pub(super) struct Worker {
    pub busy: [Mean; 2],
    pub seconds: f64,
    samples: f64,
    costs: [Option<f64>; 5],
    invalid: bool,
}

impl Default for Worker {
    fn default() -> Self {
        Self {
            busy: Default::default(),
            seconds: 0.0,
            samples: 0.0,
            costs: [Some(0.0); 5],
            invalid: false,
        }
    }
}

impl Worker {
    pub(super) fn row(&self, name: &str) -> Vec<Value> {
        let mut row = vec![
            json!(name),
            json!(self.busy[0].value()),
            json!(self.busy[1].value()),
            json!((!self.invalid).then_some(self.samples)),
        ];
        row.extend(self.costs.iter().map(|cost| {
            json!(
                cost.filter(|_| !self.invalid && self.samples > 0.0)
                    .map(|total| total * 1e6 / self.samples)
            )
        }));
        row
    }
}

#[derive(Default)]
pub(super) struct Measurements {
    pub busy: [Mean; 4],
    pub rate: Mean,
    pub workers: BTreeMap<(bool, String), Worker>,
    pub timings: BTreeMap<&'static str, (u64, f64)>,
}

impl Measurements {
    pub(super) fn observe(&mut self, interval: &Interval<'_>, fraction: f64, whole: bool) {
        let evaluator = interval.evaluator;
        let worker = self
            .workers
            .entry((
                evaluator,
                interval.last["worker_id"].as_str().unwrap().into(),
            ))
            .or_default();
        let count = interval.delta(if evaluator {
            "/samples_evaluated"
        } else {
            "/completed_samples_total"
        });
        let seconds = interval.seconds * fraction;
        worker.seconds += seconds;
        worker.samples += count.unwrap_or(0.0) * fraction;
        worker.invalid |= count.is_none();
        for (index, lane) in ["compute", "io"].into_iter().enumerate() {
            let value = count.and_then(|_| interval.busy_percent(lane));
            let weight = interval.busy_seconds().unwrap_or(interval.seconds) * fraction;
            self.busy[usize::from(!evaluator) * 2 + index].add(value, weight);
            worker.busy[index].add(value, weight);
        }
        if evaluator {
            for (index, key) in [
                "evaluate_seconds",
                "materialize_seconds",
                "fetch_wait_seconds",
                "submit_seconds",
                "submit_wait_seconds",
            ]
            .iter()
            .enumerate()
            {
                worker.costs[index] = worker.costs[index]
                    .zip(interval.delta(&format!("/cumulative/{key}")))
                    .map(|(total, delta)| total + delta * fraction);
            }
        } else {
            self.rate.add(count.map(|n| n / interval.seconds), seconds);
            // These are reset-on-publication observations, not cumulative counters.
            // Only include complete reporting intervals; never invent fractional operations.
            if whole && count.is_some() {
                for &(path, _, _) in SAMPLER_TIMINGS {
                    let metric = data(interval.last, false)
                        .pointer(path)
                        .unwrap_or(&Value::Null);
                    if let (Some(n), Some(total)) =
                        (metric["count"].as_u64(), finite(&metric["total"]))
                    {
                        let entry = self.timings.entry(path).or_default();
                        entry.0 = entry.0.saturating_add(n);
                        entry.1 += total;
                    }
                }
            }
        }
    }
}
