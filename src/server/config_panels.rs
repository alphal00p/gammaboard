use crate::core::{EngineError, EvaluatorConfig, SamplerAggregatorConfig};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, key_value,
    key_value_panel, panel_spec, replace_panel, with_panel_width,
};
use crate::utils::domain::Domain;
use serde_json::Value as JsonValue;
use serde_json::json;

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
        vec![
            with_panel_width(
                panel_spec(
                    "evaluator_summary",
                    "Evaluator Summary",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
            ),
            with_panel_width(
                panel_spec(
                    "evaluator_config",
                    "Evaluator Config",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
            ),
        ]
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
            key_value(
                "min_tick_time_ms",
                "Min Tick Time (ms)",
                ctx.runner_params.min_tick_time_ms,
            ),
            key_value(
                "db_pool_size",
                "DB Pool Size",
                ctx.runner_params.db_pool_size,
            ),
        ];
        if let Some(accumulator_kind) = evaluator_accumulator_kind(self) {
            summary.insert(
                3,
                key_value(
                    "accumulator_kind",
                    "Accumulator Kind",
                    match accumulator_kind {
                        crate::evaluation::SemanticAccumulatorKind::Scalar => "scalar",
                        crate::evaluation::SemanticAccumulatorKind::Vector => "vector",
                    },
                ),
            );
        }
        let config_payload = json!({
            "evaluator": self,
            "runner": {
                "performance_snapshot_interval_ms": ctx.runner_params.performance_snapshot_interval_ms,
                "min_tick_time_ms": ctx.runner_params.min_tick_time_ms,
                "db_pool_size": ctx.runner_params.db_pool_size,
            },
        });
        let mut panels = vec![key_value_panel("evaluator_summary", summary)];
        if let Some(config_panel) = json_value_panel("evaluator_config", &config_payload)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

fn evaluator_accumulator_kind(
    config: &EvaluatorConfig,
) -> Option<crate::evaluation::SemanticAccumulatorKind> {
    match config {
        EvaluatorConfig::Gammaloop { .. } => None,
        EvaluatorConfig::Unit { params } => Some(params.accumulator_kind),
        EvaluatorConfig::Symbolica { .. } => {
            Some(crate::evaluation::SemanticAccumulatorKind::Scalar)
        }
        EvaluatorConfig::ProcessEvaluator { .. } => {
            Some(crate::evaluation::SemanticAccumulatorKind::Scalar)
        }
    }
}

impl PanelRenderer<SamplerAggregatorPanelContext<'_>> for SamplerAggregatorConfig {
    fn panel_specs(&self, _ctx: &SamplerAggregatorPanelContext<'_>) -> Vec<PanelSpec> {
        vec![
            with_panel_width(
                panel_spec(
                    "sampler_summary",
                    "Sampler Aggregator Summary",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
            ),
            with_panel_width(
                panel_spec(
                    "sampler_config",
                    "Sampler Aggregator Config",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Full,
            ),
        ]
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
                    "snapshot_interval_ms",
                    "Snapshot Interval (ms)",
                    ctx.runner_params.performance_snapshot_interval_ms,
                ),
                key_value(
                    "min_tick_time_ms",
                    "Min Tick Time (ms)",
                    ctx.runner_params.min_tick_time_ms,
                ),
                key_value(
                    "frontend_sync_interval_ms",
                    "Frontend Sync Interval (ms)",
                    ctx.runner_params.frontend_sync_interval_ms,
                ),
                key_value(
                    "db_pool_size",
                    "DB Pool Size",
                    ctx.runner_params.db_pool_size,
                ),
            ],
        )];
        let config_payload = json!({
            "sampler_aggregator": self,
            "runner": {
                "performance_snapshot_interval_ms": ctx.runner_params.performance_snapshot_interval_ms,
                "min_tick_time_ms": ctx.runner_params.min_tick_time_ms,
                "frontend_sync_interval_ms": ctx.runner_params.frontend_sync_interval_ms,
                "db_pool_size": ctx.runner_params.db_pool_size,
                "queue": ctx.runner_params.queue,
            },
        });
        if let Some(config_panel) = json_value_panel("sampler_config", &config_payload)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

fn summarize_domain(domain: &Domain) -> String {
    match domain {
        Domain::Continuous { dims } => format!("continuous({dims})"),
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => format!("rectangular({discrete_cardinalities:?}; continuous={continuous_dims})"),
        Domain::Discrete {
            axis_label,
            branches,
        } => {
            let axis = axis_label.as_deref().unwrap_or("discrete");
            format!("{axis}[{}]", branches.len())
        }
    }
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
