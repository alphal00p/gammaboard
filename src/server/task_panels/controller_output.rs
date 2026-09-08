use crate::core::{ControllerChildOutput, MeasurementResult, TaskMeasurementOutput};
use serde_json::{Value as JsonValue, json};

pub(super) fn measurement_results(child: &ControllerChildOutput) -> Option<&[MeasurementResult]> {
    match child.measurement.as_ref()? {
        TaskMeasurementOutput::Completed { results } => Some(results),
        TaskMeasurementOutput::Failed { .. } => None,
    }
}

pub(super) fn measurement_sample_count(child: &ControllerChildOutput) -> Option<i64> {
    measurement_results(child)?
        .iter()
        .map(|result| result.sample_count)
        .max()
}

pub(super) fn select_run_payload() -> JsonValue {
    json!({ "row_action": { "kind": "select_run", "column": "run" } })
}
