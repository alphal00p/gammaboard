//! Tracing initialization and DB-backed runtime log sink.

use crate::core::{RuntimeLogEvent, RuntimeLogStore};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::mpsc;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::{
    Layer,
    filter::LevelFilter,
    layer::{Context, Filter},
    prelude::*,
    registry::LookupSpan,
    util::SubscriberInitExt,
};

const DEFAULT_CHANNEL_CAPACITY: usize = 4_096;
const DEFAULT_DB_GAMMABOARD_LOG_LEVEL: &str = "info";
const DEFAULT_DB_EXTERNAL_LOG_LEVEL: &str = "warn";

#[derive(Debug, Default, Clone)]
struct RuntimeContext {
    source: Option<String>,
    run_id: Option<i32>,
    node_uuid: Option<String>,
    node_name: Option<String>,
}

impl RuntimeContext {
    fn is_empty(&self) -> bool {
        self.source.is_none()
            && self.run_id.is_none()
            && self.node_uuid.is_none()
            && self.node_name.is_none()
    }

    fn merge_from(&mut self, other: &RuntimeContext) {
        if let Some(value) = &other.source {
            self.source = Some(value.clone());
        }
        if let Some(value) = other.run_id {
            self.run_id = Some(value);
        }
        if let Some(value) = &other.node_uuid {
            self.node_uuid = Some(value.clone());
        }
        if let Some(value) = &other.node_name {
            self.node_name = Some(value.clone());
        }
    }
}

#[derive(Debug, Clone)]
struct DbLogLayer {
    tx: Arc<mpsc::Sender<RuntimeLogEvent>>,
    dropped_events: Arc<AtomicU64>,
}

impl DbLogLayer {
    fn new(tx: mpsc::Sender<RuntimeLogEvent>) -> Self {
        Self {
            tx: Arc::new(tx),
            dropped_events: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SpanLevelPolicy {
    gammaboard: Option<LevelFilter>,
    external: Option<LevelFilter>,
}

impl SpanLevelPolicy {
    fn db_from_env() -> Self {
        Self {
            gammaboard: parse_db_level_env(
                "GAMMABOARD_DB_LOG_LEVEL",
                DEFAULT_DB_GAMMABOARD_LOG_LEVEL,
            ),
            external: parse_db_level_env(
                "GAMMABOARD_DB_EXTERNAL_LOG_LEVEL",
                DEFAULT_DB_EXTERNAL_LOG_LEVEL,
            ),
        }
    }

    fn fmt_from_quiet(quiet: bool) -> Self {
        if quiet {
            Self {
                gammaboard: Some(LevelFilter::WARN),
                external: Some(LevelFilter::WARN),
            }
        } else {
            Self {
                gammaboard: Some(LevelFilter::INFO),
                external: Some(LevelFilter::WARN),
            }
        }
    }

    fn threshold_for_target(&self, target: &str) -> Option<LevelFilter> {
        if is_gammaboard_target(target) {
            self.gammaboard
        } else {
            self.external
        }
    }

    fn allows(&self, level: &Level, target: &str) -> bool {
        let Some(threshold) = self.threshold_for_target(target) else {
            return false;
        };
        level_to_filter(level) <= threshold
    }

    fn max_threshold(&self) -> Option<LevelFilter> {
        match (self.gammaboard, self.external) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SpanLevelFilter {
    policy: SpanLevelPolicy,
}

impl SpanLevelFilter {
    fn new(policy: SpanLevelPolicy) -> Self {
        Self { policy }
    }
}

impl<S> Filter<S> for SpanLevelFilter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: &Context<'_, S>) -> bool {
        // Keep span callbacks enabled so context fields (source/run_id/worker_id)
        // are captured for descendant events.
        if metadata.is_span() {
            return true;
        }

        let Some(max_threshold) = self.policy.max_threshold() else {
            return false;
        };
        metadata.level() <= &max_threshold
    }

    fn event_enabled(&self, event: &Event<'_>, _ctx: &Context<'_, S>) -> bool {
        self.policy
            .allows(event.metadata().level(), event.metadata().target())
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        // We rely on TRACE-level context spans (source/run_id/worker_id).
        // Returning INFO/WARN here can globally disable those spans at callsite
        // registration time, which strips context and causes DB log drops.
        self.policy.max_threshold().map(|_| LevelFilter::TRACE)
    }
}

fn is_gammaboard_target(target: &str) -> bool {
    target == "gammaboard" || target.starts_with("gammaboard::")
}

fn level_to_filter(level: &Level) -> LevelFilter {
    match *level {
        Level::ERROR => LevelFilter::ERROR,
        Level::WARN => LevelFilter::WARN,
        Level::INFO => LevelFilter::INFO,
        Level::DEBUG => LevelFilter::DEBUG,
        Level::TRACE => LevelFilter::TRACE,
    }
}

fn context_from_scope<S>(ctx: &Context<'_, S>, event: &Event<'_>) -> RuntimeContext
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut context = RuntimeContext::default();
    if let Some(scope) = ctx.event_scope(event) {
        for span in scope.from_root() {
            if let Some(stored) = span.extensions().get::<RuntimeContext>() {
                context.merge_from(stored);
            }
        }
    }
    context
}

impl DbLogLayer {
    fn update_span_context<S>(
        ctx: Context<'_, S>,
        id: &tracing::span::Id,
        record_fields: impl FnOnce(&mut JsonFieldVisitor),
    ) where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let Some(span) = ctx.span(id) else {
            return;
        };

        let mut visitor = JsonFieldVisitor::default();
        record_fields(&mut visitor);
        let mut fields = visitor.fields;
        let context_update = extract_context_update(&mut fields);
        if context_update.is_empty() {
            return;
        }

        let mut extensions = span.extensions_mut();
        if let Some(existing) = extensions.get_mut::<RuntimeContext>() {
            existing.merge_from(&context_update);
        } else {
            extensions.insert(context_update);
        }
    }
}

impl<S> Layer<S> for DbLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        Self::update_span_context(ctx, id, |visitor| attrs.record(visitor));
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        Self::update_span_context(ctx, id, |visitor| values.record(visitor));
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut context = context_from_scope(&ctx, event);
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let mut fields = visitor.fields;
        let event_context = extract_context_update(&mut fields);
        context.merge_from(&event_context);

        let Some(source) = context.source else {
            return;
        };
        let message = extract_message(&mut fields).unwrap_or_else(|| metadata.name().to_string());

        let record = RuntimeLogEvent {
            source,
            run_id: context.run_id,
            node_uuid: context.node_uuid,
            node_name: context.node_name,
            level: metadata.level().to_string().to_lowercase(),
            target: metadata.target().to_string(),
            message,
            fields: JsonValue::Object(fields),
        };

        if let Err(err) = self.tx.try_send(record)
            && matches!(err, mpsc::error::TrySendError::Full(_))
        {
            let dropped = self.dropped_events.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped.is_power_of_two() {
                tracing::warn!(
                    target: "telemetry",
                    dropped_events = dropped,
                    "runtime log channel is full; dropping log events"
                );
            }
        }
    }
}

fn parse_db_level_env(var_name: &str, default: &str) -> Option<LevelFilter> {
    let raw = std::env::var(var_name).unwrap_or_else(|_| default.to_string());
    parse_level_filter_or_off(&raw).unwrap_or_else(|| {
        eprintln!(
            "invalid {var_name}={raw:?}; expected one of off,error,warn,info,debug,trace; using default {default}"
        );
        parse_level_filter_or_off(default).unwrap_or(Some(LevelFilter::INFO))
    })
}

fn parse_level_filter_or_off(value: &str) -> Option<Option<LevelFilter>> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "off" => Some(None),
        "error" => Some(Some(LevelFilter::ERROR)),
        "warn" | "warning" => Some(Some(LevelFilter::WARN)),
        "info" => Some(Some(LevelFilter::INFO)),
        "debug" => Some(Some(LevelFilter::DEBUG)),
        "trace" => Some(Some(LevelFilter::TRACE)),
        _ => None,
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: JsonMap<String, JsonValue>,
}

impl tracing::field::Visit for JsonFieldVisitor {
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            JsonValue::Number(JsonNumber::from(value)),
        );
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            JsonValue::Number(JsonNumber::from(value)),
        );
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), JsonValue::Bool(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields.insert(
            field.name().to_string(),
            JsonValue::String(value.to_string()),
        );
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            JsonValue::String(format!("{value:?}")),
        );
    }
}

fn remove_string(fields: &mut JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    let value = fields.remove(key)?;
    match value {
        JsonValue::String(value) => Some(value),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn remove_i32(fields: &mut JsonMap<String, JsonValue>, key: &str) -> Option<i32> {
    let value = fields.remove(key)?;
    match value {
        JsonValue::Number(number) => number.as_i64().and_then(|value| i32::try_from(value).ok()),
        JsonValue::String(value) => value.parse::<i32>().ok(),
        _ => None,
    }
}

fn extract_context_update(fields: &mut JsonMap<String, JsonValue>) -> RuntimeContext {
    RuntimeContext {
        source: remove_string(fields, "source"),
        run_id: remove_i32(fields, "run_id"),
        node_uuid: remove_string(fields, "node_uuid").or_else(|| remove_string(fields, "node_id")),
        node_name: remove_string(fields, "node_name")
            .or_else(|| remove_string(fields, "worker_id")),
    }
}

fn extract_message(fields: &mut JsonMap<String, JsonValue>) -> Option<String> {
    remove_string(fields, "message")
}

async fn write_runtime_logs<S>(store: S, mut rx: mpsc::Receiver<RuntimeLogEvent>)
where
    S: RuntimeLogStore + Send + Sync + 'static,
{
    while let Some(record) = rx.recv().await {
        if let Err(err) = store.insert_runtime_log(&record).await {
            tracing::warn!(
                target: "telemetry",
                error = %err,
                "failed to persist runtime log event"
            );
        }
    }
}

/// Initialize global tracing subscriber.
///
/// If `runtime_log_store` is provided, context-tagged runtime events are
/// persisted through that store.
pub fn init_tracing<S>(
    runtime_log_store: Option<S>,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: RuntimeLogStore + Send + Sync + 'static,
{
    if let Some(store) = runtime_log_store {
        let (tx, rx) = mpsc::channel::<RuntimeLogEvent>(DEFAULT_CHANNEL_CAPACITY);
        tokio::spawn(write_runtime_logs(store, rx));

        let db_filter = SpanLevelFilter::new(SpanLevelPolicy::db_from_env());
        let db_layer = DbLogLayer::new(tx).with_filter(db_filter);
        let fmt_filter = SpanLevelFilter::new(SpanLevelPolicy::fmt_from_quiet(quiet));
        let fmt_layer = tracing_subscriber::fmt::layer().with_filter(fmt_filter);
        tracing_subscriber::registry()
            .with(db_layer)
            .with(fmt_layer)
            .try_init()?;
    } else {
        let fmt_filter = SpanLevelFilter::new(SpanLevelPolicy::fmt_from_quiet(quiet));
        let fmt_layer = tracing_subscriber::fmt::layer().with_filter(fmt_filter);
        tracing_subscriber::registry().with(fmt_layer).try_init()?;
    }
    Ok(())
}
