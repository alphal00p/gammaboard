use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GammaLoopDiagnostics {
    pub count_total: i64,
    pub count_double_precision: i64,
    pub count_quad_precision: i64,
    pub count_arb_precision: i64,
    pub count_nan: i64,
    pub count_nan_or_unstable: i64,
    pub count_loop_momenta_escalated: i64,
    pub total_eval_time_ms: f64,
    pub total_integrand_eval_time_ms: f64,
    pub total_evaluator_eval_time_ms: f64,
    pub total_parameterization_time_ms: f64,
    pub total_event_processing_time_ms: f64,
    /// Inclusive subsets of sampling and physical time. Absent in older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_canonical_sampling_preparation_time_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_canonical_physical_preparation_time_ms: Option<f64>,
    pub total_generated_events: i64,
    pub total_accepted_events: i64,
}

impl GammaLoopDiagnostics {
    pub fn merge_in_place(&mut self, other: Self) {
        if other.count_total > 0 {
            for (total, incoming) in [
                (
                    &mut self.total_canonical_sampling_preparation_time_ms,
                    other.total_canonical_sampling_preparation_time_ms,
                ),
                (
                    &mut self.total_canonical_physical_preparation_time_ms,
                    other.total_canonical_physical_preparation_time_ms,
                ),
            ] {
                *total = if self.count_total == 0 {
                    incoming
                } else {
                    total.zip(incoming).map(|(a, b)| a + b)
                };
            }
        }
        self.count_total += other.count_total;
        self.count_double_precision += other.count_double_precision;
        self.count_quad_precision += other.count_quad_precision;
        self.count_arb_precision += other.count_arb_precision;
        self.count_nan += other.count_nan;
        self.count_nan_or_unstable += other.count_nan_or_unstable;
        self.count_loop_momenta_escalated += other.count_loop_momenta_escalated;
        self.total_eval_time_ms += other.total_eval_time_ms;
        self.total_integrand_eval_time_ms += other.total_integrand_eval_time_ms;
        self.total_evaluator_eval_time_ms += other.total_evaluator_eval_time_ms;
        self.total_parameterization_time_ms += other.total_parameterization_time_ms;
        self.total_event_processing_time_ms += other.total_event_processing_time_ms;
        self.total_generated_events += other.total_generated_events;
        self.total_accepted_events += other.total_accepted_events;
    }

    pub fn avg_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_eval_time_ms, self.count_total)
    }

    pub fn avg_integrand_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_integrand_eval_time_ms, self.count_total)
    }

    pub fn avg_evaluator_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_evaluator_eval_time_ms, self.count_total)
    }

    pub fn avg_parameterization_time_ms(&self) -> f64 {
        safe_ratio(self.total_parameterization_time_ms, self.count_total)
    }

    pub fn avg_event_processing_time_ms(&self) -> f64 {
        safe_ratio(self.total_event_processing_time_ms, self.count_total)
    }

    pub fn promoted_to_quad_ratio(&self) -> f64 {
        safe_ratio(self.count_quad_precision as f64, self.count_total)
    }

    pub fn promoted_to_arb_ratio(&self) -> f64 {
        safe_ratio(self.count_arb_precision as f64, self.count_total)
    }

    pub fn nan_or_unstable_ratio(&self) -> f64 {
        safe_ratio(self.count_nan_or_unstable as f64, self.count_total)
    }

    pub fn loop_momenta_escalated_ratio(&self) -> f64 {
        safe_ratio(self.count_loop_momenta_escalated as f64, self.count_total)
    }

    pub fn accepted_event_ratio(&self) -> f64 {
        safe_ratio(
            self.total_accepted_events as f64,
            self.total_generated_events,
        )
    }
}

fn safe_ratio(numerator: f64, denominator: i64) -> f64 {
    if denominator > 0 {
        numerator / denominator as f64
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_timings_merge_without_inventing_coverage_for_old_samples() {
        let measured = GammaLoopDiagnostics {
            count_total: 2,
            total_canonical_sampling_preparation_time_ms: Some(3.0),
            total_canonical_physical_preparation_time_ms: Some(5.0),
            ..Default::default()
        };
        let mut total = GammaLoopDiagnostics::default();
        total.merge_in_place(measured.clone());
        total.merge_in_place(measured.clone());
        assert_eq!(total.count_total, 4);
        assert_eq!(
            total.total_canonical_sampling_preparation_time_ms,
            Some(6.0)
        );
        assert_eq!(
            total.total_canonical_physical_preparation_time_ms,
            Some(10.0)
        );
        total.merge_in_place(GammaLoopDiagnostics::default());
        assert_eq!(
            total.total_canonical_sampling_preparation_time_ms,
            Some(6.0)
        );
        let old = GammaLoopDiagnostics {
            count_total: 1,
            ..Default::default()
        };
        let encoded = serde_json::to_value(&old).unwrap();
        assert!(
            encoded
                .get("total_canonical_sampling_preparation_time_ms")
                .is_none()
        );
        total.merge_in_place(serde_json::from_value(encoded).unwrap());
        total.merge_in_place(measured);
        assert_eq!(total.count_total, 7);
        assert!(total.total_canonical_sampling_preparation_time_ms.is_none());
        assert!(total.total_canonical_physical_preparation_time_ms.is_none());
    }
}
