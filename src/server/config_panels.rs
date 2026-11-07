use crate::core::{EngineError, EvaluatorConfig, SamplingConfig};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelSpec, PanelState, PanelWidth, key_value, key_value_panel,
    sized_panel_spec,
};
use crate::utils::domain::Domain;
use serde_json::Value as JsonValue;
use serde_json::json;

pub trait PanelProvider<C> {
    fn panel_specs(&self, ctx: &C) -> Vec<PanelSpec>;
    fn panel_states(&self, ctx: &C) -> Result<Vec<PanelState>, EngineError>;
}

pub struct EvaluatorPanelContext<'a> {
    pub domain: &'a Domain,
    pub runner_params: &'a EvaluatorRunnerParams,
    pub provenance: &'a str,
}

pub struct SamplerAggregatorPanelContext<'a> {
    pub domain: &'a Domain,
    pub runner_params: &'a SamplerAggregatorRunnerParams,
    pub provenance: &'a str,
}

impl PanelProvider<EvaluatorPanelContext<'_>> for EvaluatorConfig {
    fn panel_specs(&self, _ctx: &EvaluatorPanelContext<'_>) -> Vec<PanelSpec> {
        vec![
            sized_panel_spec(
                "evaluator_summary",
                "Evaluator Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
            sized_panel_spec(
                "evaluator_config",
                "Evaluator Config",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
        ]
    }

    fn panel_states(
        &self,
        ctx: &EvaluatorPanelContext<'_>,
    ) -> Result<Vec<PanelState>, EngineError> {
        let db_pool_size = crate::runners::MAX_EVALUATOR_DB_CONNECTIONS;
        let summary = vec![
            key_value("implementation", "Implementation", self.kind_str()),
            key_value("effective_source", "Effective Source", ctx.provenance),
            key_value("domain", "Domain", summarize_domain(ctx.domain)),
            key_value(
                "snapshot_interval_ms",
                "Snapshot Interval (ms)",
                ctx.runner_params.performance_snapshot_interval_ms,
            ),
            key_value("db_pool_size", "Effective DB Pool Size", db_pool_size),
        ];
        let config_payload = json!({
            "evaluator": self,
            "runner": {
                "performance_snapshot_interval_ms": ctx.runner_params.performance_snapshot_interval_ms,
                "db_pool_size": db_pool_size,
            },
        });
        let mut panels = vec![key_value_panel("evaluator_summary", summary)];
        if let Some(config_panel) = json_value_panel("evaluator_config", &config_payload)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

impl PanelProvider<SamplerAggregatorPanelContext<'_>> for SamplingConfig {
    fn panel_specs(&self, _ctx: &SamplerAggregatorPanelContext<'_>) -> Vec<PanelSpec> {
        vec![
            sized_panel_spec(
                "sampler_summary",
                "Sampler Aggregator Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
            sized_panel_spec(
                "sampler_config",
                "Sampler Aggregator Config",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
        ]
    }

    fn panel_states(
        &self,
        ctx: &SamplerAggregatorPanelContext<'_>,
    ) -> Result<Vec<PanelState>, EngineError> {
        let db_pool_size = crate::runners::role_db_pool_size(
            crate::core::WorkerRole::SamplerAggregator,
            ctx.runner_params.db_pool_size,
        );
        let mut panels = vec![key_value_panel(
            "sampler_summary",
            vec![
                key_value("implementation", "Implementation", self.kind_str()),
                key_value("effective_source", "Effective Source", ctx.provenance),
                key_value(
                    "domain",
                    "Sampling Domain",
                    summarize_domain(&self.sampling_domain(ctx.domain)?),
                ),
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
                    "io_threads",
                    "I/O Threads",
                    ctx.runner_params.io_threads.get(),
                ),
                key_value(
                    "frontend_sync_interval_ms",
                    "Frontend Sync Interval (ms)",
                    ctx.runner_params.frontend_sync_interval_ms,
                ),
                key_value("db_pool_size", "Effective DB Pool Size", db_pool_size),
            ],
        )];
        let config_payload = json!({
            "sampler": self,
            "runner": {
                "performance_snapshot_interval_ms": ctx.runner_params.performance_snapshot_interval_ms,
                "min_tick_time_ms": ctx.runner_params.min_tick_time_ms,
                "io_threads": ctx.runner_params.io_threads,
                "frontend_sync_interval_ms": ctx.runner_params.frontend_sync_interval_ms,
                "db_pool_size": db_pool_size,
                "queue": ctx.runner_params.queue,
            },
        });
        if let Some(config_panel) = json_value_panel("sampler_config", &config_payload)? {
            panels.push(config_panel);
        }
        Ok(panels)
    }
}

fn summarize_domain(domain: &Domain) -> JsonValue {
    json!(domain)
}

fn json_value_panel(panel_id: &str, value: &JsonValue) -> Result<Option<PanelState>, EngineError> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    // Keep arrays, objects, and nulls intact for the dashboard's configuration
    // tree instead of flattening paths or turning arrays into display strings.
    let entries = object
        .iter()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_panels_preserve_nested_objects_arrays_and_nulls() {
        let source = json!({"evaluator": {"kind": "symbolica", "args": ["x", "y"],
            "source": {"expressions": [{"name": "real", "value": null}]}, "enabled": false},
            "runner": {"limits": {"batch": 32}, "empty": []}});
        let Some(PanelState::KeyValue { entries, .. }) =
            json_value_panel("evaluator_config", &source).unwrap()
        else {
            panic!("configuration")
        };
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert_eq!(entry.value, source[&entry.key]);
        }
    }
}
