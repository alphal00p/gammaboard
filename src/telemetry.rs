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
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer, filter::LevelFilter, layer::Context, prelude::*, registry::LookupSpan,
    util::SubscriberInitExt,
};

const WORKER_LOG_TARGET: &str = "worker_log";
const DEFAULT_CHANNEL_CAPACITY: usize = 4_096;

#[derive(Debug, Default, Clone)]
struct RuntimeContext {
    source: Option<String>,
    run_id: Option<i32>,
    worker_id: Option<String>,
    engine: Option<bool>,
}

impl RuntimeContext {
    fn is_empty(&self) -> bool {
        self.source.is_none()
            && self.run_id.is_none()
            && self.worker_id.is_none()
            && self.engine.is_none()
    }

    fn merge_from(&mut self, other: &RuntimeContext) {
        if let Some(value) = &other.source {
            self.source = Some(value.clone());
        }
        if let Some(value) = other.run_id {
            self.run_id = Some(value);
        }
        if let Some(value) = &other.worker_id {
            self.worker_id = Some(value.clone());
        }
        if let Some(value) = other.engine {
            self.engine = Some(value);
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

    fn should_persist(level: &Level, source: &str, engine: bool) -> bool {
        let threshold = match (source, engine) {
            (_, true) => Level::WARN,
            _ => Level::INFO,
        };
        level <= &threshold
    }

    fn context_from_scope<S>(ctx: Context<'_, S>, event: &Event<'_>) -> RuntimeContext
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

    fn update_span_context<S>(
        ctx: Context<'_, S>,
        id: &tracing::span::Id,
        record_fields: impl FnOnce(&mut JsonVisitorMut<'_>),
    ) where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let Some(span) = ctx.span(id) else {
            return;
        };

        let mut fields = JsonMap::new();
        {
            let mut visitor = JsonVisitorMut {
                fields: &mut fields,
            };
            record_fields(&mut visitor);
        }
        let update = extract_context_update(&mut fields);
        if update.is_empty() {
            return;
        }

        let mut extensions = span.extensions_mut();
        if let Some(existing) = extensions.get_mut::<RuntimeContext>() {
            existing.merge_from(&update);
        } else {
            extensions.insert(update);
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

        let mut context = Self::context_from_scope(ctx, event);

        // Backward-compatible path for explicit worker-target logs.
        if context.source.is_none() && metadata.target() == WORKER_LOG_TARGET {
            context.source = Some("worker".to_string());
        }

        // Fast path: if span context already determines this event is filtered out,
        // skip recording event fields entirely.
        if let Some(source) = context.source.as_deref() {
            let engine = context.engine.unwrap_or(false);
            if !Self::should_persist(metadata.level(), source, engine) {
                return;
            }
        }

        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        context.merge_from(&visitor.context_update);

        // Backward-compatible path for explicit worker-target logs.
        if context.source.is_none() && metadata.target() == WORKER_LOG_TARGET {
            context.source = Some("worker".to_string());
        }

        let Some(source) = context.source else {
            return;
        };
        let engine = context.engine.unwrap_or(false);
        if !Self::should_persist(metadata.level(), &source, engine) {
            return;
        }

        let message = visitor
            .message
            .unwrap_or_else(|| metadata.name().to_string());

        let record = RuntimeLogEvent {
            source,
            run_id: context.run_id,
            worker_id: context.worker_id,
            level: metadata.level().to_string().to_lowercase(),
            target: metadata.target().to_string(),
            message,
            fields: JsonValue::Object(visitor.fields),
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

struct JsonVisitorMut<'a> {
    fields: &'a mut JsonMap<String, JsonValue>,
}

#[derive(Default)]
struct EventVisitor {
    context_update: RuntimeContext,
    message: Option<String>,
    fields: JsonMap<String, JsonValue>,
}

impl EventVisitor {
    fn handle_i64(&mut self, key: &str, value: i64) -> bool {
        match key {
            "run_id" => {
                if let Ok(run_id) = i32::try_from(value) {
                    self.context_update.run_id = Some(run_id);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn handle_u64(&mut self, key: &str, value: u64) -> bool {
        match key {
            "run_id" => {
                if let Ok(run_id) = i32::try_from(value) {
                    self.context_update.run_id = Some(run_id);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn handle_bool(&mut self, key: &str, value: bool) -> bool {
        match key {
            "engine" => {
                self.context_update.engine = Some(value);
                true
            }
            _ => false,
        }
    }

    fn handle_str(&mut self, key: &str, value: &str) -> bool {
        match key {
            "source" => {
                self.context_update.source = Some(value.to_string());
                true
            }
            "worker_id" => {
                self.context_update.worker_id = Some(value.to_string());
                true
            }
            "message" => {
                self.message = Some(value.to_string());
                true
            }
            "run_id" => {
                if let Ok(run_id) = value.parse::<i32>() {
                    self.context_update.run_id = Some(run_id);
                    true
                } else {
                    false
                }
            }
            "engine" => {
                if let Ok(engine) = value.parse::<bool>() {
                    self.context_update.engine = Some(engine);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl tracing::field::Visit for JsonVisitorMut<'_> {
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

impl tracing::field::Visit for EventVisitor {
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        let key = field.name();
        if !self.handle_i64(key, value) {
            self.fields
                .insert(key.to_string(), JsonValue::Number(JsonNumber::from(value)));
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        let key = field.name();
        if !self.handle_u64(key, value) {
            self.fields
                .insert(key.to_string(), JsonValue::Number(JsonNumber::from(value)));
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        let key = field.name();
        if !self.handle_bool(key, value) {
            self.fields.insert(key.to_string(), JsonValue::Bool(value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let key = field.name();
        if !self.handle_str(key, value) {
            self.fields
                .insert(key.to_string(), JsonValue::String(value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let key = field.name();
        let debug_str = format!("{value:?}");
        let unquoted = unquote_json_string(debug_str);
        if !self.handle_str(key, &unquoted) {
            self.fields
                .insert(key.to_string(), JsonValue::String(unquoted));
        }
    }
}

fn unquote_json_string(value: String) -> String {
    if value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str::<String>(&value).unwrap_or(value);
    }
    value
}

fn remove_string(fields: &mut JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    let value = fields.remove(key)?;
    match value {
        JsonValue::String(value) => Some(unquote_json_string(value)),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn remove_i32(fields: &mut JsonMap<String, JsonValue>, key: &str) -> Option<i32> {
    let value = fields.remove(key)?;
    match value {
        JsonValue::Number(number) => number.as_i64().and_then(|value| i32::try_from(value).ok()),
        JsonValue::String(value) => unquote_json_string(value).parse::<i32>().ok(),
        _ => None,
    }
}

fn remove_bool(fields: &mut JsonMap<String, JsonValue>, key: &str) -> Option<bool> {
    let value = fields.remove(key)?;
    match value {
        JsonValue::Bool(value) => Some(value),
        JsonValue::String(value) => unquote_json_string(value).parse::<bool>().ok(),
        _ => None,
    }
}

fn extract_context_update(fields: &mut JsonMap<String, JsonValue>) -> RuntimeContext {
    RuntimeContext {
        source: remove_string(fields, "source"),
        run_id: remove_i32(fields, "run_id"),
        worker_id: remove_string(fields, "worker_id"),
        engine: remove_bool(fields, "engine"),
    }
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
pub fn init_tracing<S>(runtime_log_store: Option<S>) -> Result<(), Box<dyn std::error::Error>>
where
    S: RuntimeLogStore + Send + Sync + 'static,
{
    let fmt_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(fmt_filter);
    if let Some(store) = runtime_log_store {
        let (tx, rx) = mpsc::channel::<RuntimeLogEvent>(DEFAULT_CHANNEL_CAPACITY);
        tokio::spawn(write_runtime_logs(store, rx));

        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(DbLogLayer::new(tx).with_filter(LevelFilter::TRACE))
            .try_init()?;
    } else {
        tracing_subscriber::registry().with(fmt_layer).try_init()?;
    }
    Ok(())
}
