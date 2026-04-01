use crate::core::{EngineError, EvaluatorConfig, SamplerAggregatorConfig};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, key_value,
    key_value_panel, panel_spec, replace_panel, with_panel_width,
};
use crate::utils::domain::Domain;
use serde::Serialize;
use serde_json::Value as JsonValue;

pub trait PanelRenderer<C> {
    fn panel_specs(&self, ctx: &C) -> Vec<PanelSpec>;
    fn panel_states(&self, ctx: &C) -> Result<Vec<PanelState>, EngineError>;

    fn build_response(&self, source_id: String, ctx: &C) -> Result<PanelResponse, EngineError> {
        Ok(PanelResponse {
            source_id,
            cursor: None,
            reset_required: false,
            panels: self.panel_specs(ctx),
            updates: self
                .panel_states(ctx)?
                .into_iter()
                .map(replace_panel)
                .collect(),
        })
    }
}

pub struct EvaluatorPanelContext<'a> {
    pub domain: &'a Domain,
    pub runner_params: &'a EvaluatorRunnerParams,
}

pub struct SamplerAggregatorPanelContext<'a> {
    pub domain: &'a Domain,
    pub runner_params: &'a SamplerAggregatorRunnerParams,
}

impl PanelRenderer<EvaluatorPanelContext<'_>> for EvaluatorConfig {
    fn panel_specs(&self, _ctx: &EvaluatorPanelContext<'_>) -> Vec<PanelSpec> {
        let mut panels = vec![panel_spec(
            "evaluator_summary",
            "Evaluator Summary",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        )];
        panels[0].width = PanelWidth::Half;
        if has_object_fields(self) {
            panels.push(with_panel_width(
                panel_spec(
                    "evaluator_config",
                    "Evaluator Config",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
            ));
        }
        panels
    }

    fn panel_states(
        &self,
        ctx: &EvaluatorPanelContext<'_>,
    ) -> Result<Vec<PanelState>, EngineError> {
        let mut summary = vec![
            key_value("implementation", "Implementation", self.kind_str()),
            key_value("domain", "Domain", summarize_domain(ctx.domain)),
            key_value(
                "snapshot_interval_ms",
                "Snapshot Interval (ms)",
                ctx.runner_params.performance_snapshot_interval_ms,
            ),
        ];
        if let Some(observable_kind) = evaluator_observable_kind(self) {
            summary.insert(
                3,
                key_value(
                    "observable_kind",
                    "Observable Kind",
                    match observable_kind {
                        crate::evaluation::SemanticObservableKind::Scalar => "scalar",
                        crate::evaluation::SemanticObservableKind::Complex => "complex",
                    },
                ),
            );
        }
        let mut panels = vec![key_value_panel("evaluator_summary", summary)];
        if let Some(config_panel) = json_object_panel("evaluator_config", self)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

fn evaluator_observable_kind(
    config: &EvaluatorConfig,
) -> Option<crate::evaluation::SemanticObservableKind> {
    match config {
        EvaluatorConfig::Gammaloop { .. } => None,
        EvaluatorConfig::SinEvaluator { .. } => {
            Some(crate::evaluation::SemanticObservableKind::Scalar)
        }
        EvaluatorConfig::SincEvaluator { .. } => {
            Some(crate::evaluation::SemanticObservableKind::Complex)
        }
        EvaluatorConfig::Unit { params } => Some(params.observable_kind),
        EvaluatorConfig::Symbolica { .. } => {
            Some(crate::evaluation::SemanticObservableKind::Scalar)
        }
    }
}

impl PanelRenderer<SamplerAggregatorPanelContext<'_>> for SamplerAggregatorConfig {
    fn panel_specs(&self, _ctx: &SamplerAggregatorPanelContext<'_>) -> Vec<PanelSpec> {
        let mut panels = vec![panel_spec(
            "sampler_summary",
            "Sampler Aggregator Summary",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        )];
        panels[0].width = PanelWidth::Half;
        if has_object_fields(self) {
            panels.push(with_panel_width(
                panel_spec(
                    "sampler_config",
                    "Sampler Aggregator Config",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
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
                key_value("implementation", "Implementation", self.kind_str()),
                key_value("domain", "Domain", summarize_domain(ctx.domain)),
                key_value(
                    "frontend_sync_interval_ms",
                    "Frontend Sync Interval (ms)",
                    ctx.runner_params.frontend_sync_interval_ms,
                ),
                key_value(
                    "queue_buffer",
                    "Queue Buffer",
                    ctx.runner_params.queue_buffer,
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
                    "strict_batch_ordering",
                    "Strict Batch Ordering",
                    ctx.runner_params.strict_batch_ordering,
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

fn summarize_domain(domain: &Domain) -> String {
    match domain {
        Domain::Continuous { dims } => format!("continuous({dims})"),
        Domain::Discrete {
            axis_label,
            branches,
        } => {
            let axis = axis_label.as_deref().unwrap_or("discrete");
            format!("{axis}[{}]", branches.len())
        }
    }
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
    let mut entries = Vec::new();
    for (key, value) in object.iter().filter(|(key, _)| key.as_str() != "kind") {
        collect_json_entries(key, &title_label(key), value, &mut entries);
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(key_value_panel(panel_id, entries)))
}

fn collect_json_entries(
    key_prefix: &str,
    label_prefix: &str,
    value: &JsonValue,
    entries: &mut Vec<crate::server::panels::KeyValueEntry>,
) {
    match value {
        JsonValue::Object(object) => {
            for (key, child) in object {
                let next_key = format!("{key_prefix}.{key}");
                let next_label = format!("{label_prefix} {}", title_label(key));
                collect_json_entries(&next_key, &next_label, child, entries);
            }
        }
        JsonValue::Array(values) => {
            let rendered = values
                .iter()
                .map(compact_json_value)
                .collect::<Vec<_>>()
                .join(", ");
            entries.push(key_value(key_prefix, label_prefix, format!("[{rendered}]")));
        }
        _ => entries.push(key_value(key_prefix, label_prefix, value.clone())),
    }
}

fn compact_json_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "none".to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.clone(),
        JsonValue::Array(values) => {
            let rendered = values
                .iter()
                .map(compact_json_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        JsonValue::Object(_) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
    }
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
