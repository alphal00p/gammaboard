use crate::core::{EngineError, EvaluatorConfig, SamplerAggregatorConfig};
use crate::evaluation::PointSpec;
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::server::panels::{
    CurrentPanelsResponse, PanelDescriptor, PanelKind, PanelState, key_value, key_value_panel,
    panel_descriptor,
};
use serde::Serialize;
use serde_json::Value as JsonValue;

pub trait CurrentPanelRenderer<C> {
    fn panel_descriptors(&self, ctx: &C) -> Vec<PanelDescriptor>;
    fn panel_states(&self, ctx: &C) -> Result<Vec<PanelState>, EngineError>;

    fn build_response(&self, ctx: &C) -> Result<CurrentPanelsResponse, EngineError> {
        Ok(CurrentPanelsResponse {
            panels: self.panel_descriptors(ctx),
            current: self.panel_states(ctx)?,
        })
    }
}

pub struct EvaluatorPanelContext<'a> {
    pub point_spec: &'a PointSpec,
    pub runner_params: &'a EvaluatorRunnerParams,
    pub init_metadata: Option<&'a JsonValue>,
}

pub struct SamplerAggregatorPanelContext<'a> {
    pub point_spec: &'a PointSpec,
    pub runner_params: &'a SamplerAggregatorRunnerParams,
}

impl CurrentPanelRenderer<EvaluatorPanelContext<'_>> for EvaluatorConfig {
    fn panel_descriptors(&self, ctx: &EvaluatorPanelContext<'_>) -> Vec<PanelDescriptor> {
        let mut panels = vec![panel_descriptor(
            "evaluator_summary",
            "Evaluator Summary",
            PanelKind::KeyValue,
            false,
        )];
        if has_object_fields(self) {
            panels.push(panel_descriptor(
                "evaluator_config",
                "Evaluator Config",
                PanelKind::KeyValue,
                false,
            ));
        }
        if ctx.init_metadata.is_some_and(json_has_object_fields) {
            panels.push(panel_descriptor(
                "evaluator_init_metadata",
                "Evaluator Init Metadata",
                PanelKind::KeyValue,
                false,
            ));
        }
        panels
    }

    fn panel_states(
        &self,
        ctx: &EvaluatorPanelContext<'_>,
    ) -> Result<Vec<PanelState>, EngineError> {
        let mut panels = vec![key_value_panel(
            "evaluator_summary",
            vec![
                key_value(
                    "implementation",
                    "Implementation",
                    implementation_kind(self),
                ),
                key_value(
                    "continuous_dims",
                    "Continuous Dims",
                    ctx.point_spec.continuous_dims,
                ),
                key_value(
                    "discrete_dims",
                    "Discrete Dims",
                    ctx.point_spec.discrete_dims,
                ),
                key_value(
                    "observable_kind",
                    "Observable Kind",
                    match self.observable_kind() {
                        crate::evaluation::SemanticObservableKind::Scalar => "scalar",
                        crate::evaluation::SemanticObservableKind::Complex => "complex",
                    },
                ),
                key_value(
                    "snapshot_interval_ms",
                    "Snapshot Interval (ms)",
                    ctx.runner_params.performance_snapshot_interval_ms,
                ),
            ],
        )];
        if let Some(config_panel) = json_object_panel("evaluator_config", self)? {
            panels.push(config_panel);
        }
        if let Some(metadata) = ctx.init_metadata {
            if let Some(metadata_panel) = json_value_panel("evaluator_init_metadata", metadata)? {
                panels.push(metadata_panel);
            }
        }
        Ok(panels)
    }
}

impl CurrentPanelRenderer<SamplerAggregatorPanelContext<'_>> for SamplerAggregatorConfig {
    fn panel_descriptors(&self, _ctx: &SamplerAggregatorPanelContext<'_>) -> Vec<PanelDescriptor> {
        let mut panels = vec![panel_descriptor(
            "sampler_summary",
            "Sampler Aggregator Summary",
            PanelKind::KeyValue,
            false,
        )];
        if has_object_fields(self) {
            panels.push(panel_descriptor(
                "sampler_config",
                "Sampler Aggregator Config",
                PanelKind::KeyValue,
                false,
            ));
        }
        panels
    }

    fn panel_states(
        &self,
        ctx: &SamplerAggregatorPanelContext<'_>,
    ) -> Result<Vec<PanelState>, EngineError> {
        let mut panels = vec![key_value_panel(
            "sampler_summary",
            vec![
                key_value(
                    "implementation",
                    "Implementation",
                    implementation_kind(self),
                ),
                key_value(
                    "continuous_dims",
                    "Continuous Dims",
                    ctx.point_spec.continuous_dims,
                ),
                key_value(
                    "discrete_dims",
                    "Discrete Dims",
                    ctx.point_spec.discrete_dims,
                ),
                key_value(
                    "target_queue_remaining",
                    "Target Queue Remaining",
                    ctx.runner_params.target_queue_remaining,
                ),
                key_value(
                    "max_batch_size",
                    "Max Batch Size",
                    ctx.runner_params.max_batch_size,
                ),
                key_value(
                    "max_queue_size",
                    "Max Queue Size",
                    ctx.runner_params.max_queue_size,
                ),
                key_value(
                    "max_batches_per_tick",
                    "Max Batches Per Tick",
                    ctx.runner_params.max_batches_per_tick,
                ),
                key_value(
                    "completed_fetch_limit",
                    "Completed Fetch Limit",
                    ctx.runner_params.completed_batch_fetch_limit,
                ),
                key_value(
                    "snapshot_interval_ms",
                    "Snapshot Interval (ms)",
                    ctx.runner_params.performance_snapshot_interval_ms,
                ),
            ],
        )];
        if let Some(config_panel) = json_object_panel("sampler_config", self)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

fn implementation_kind<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .and_then(|object| object.get("kind"))
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn has_object_fields<T: Serialize>(value: &T) -> bool {
    serde_json::to_value(value)
        .ok()
        .is_some_and(|value| json_has_object_fields(&value))
}

fn json_has_object_fields(value: &JsonValue) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.iter().any(|(key, _)| key.as_str() != "kind"))
}

fn json_object_panel<T: Serialize>(
    panel_id: &str,
    value: &T,
) -> Result<Option<PanelState>, EngineError> {
    let value = serde_json::to_value(value)
        .map_err(|err| EngineError::engine(format!("failed to serialize config panel: {err}")))?;
    json_value_panel(panel_id, &value)
}

fn json_value_panel(panel_id: &str, value: &JsonValue) -> Result<Option<PanelState>, EngineError> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let entries = object
        .iter()
        .filter(|(key, _)| key.as_str() != "kind")
        .map(|(key, value)| key_value(key, &title_label(key), value.clone()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(key_value_panel(panel_id, entries)))
}

fn title_label(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
