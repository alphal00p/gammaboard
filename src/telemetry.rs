//! Tracing initialization and DB-backed worker log sink.

use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use sqlx::PgPool;
use std::{fmt, sync::Arc};
use tokio::sync::mpsc;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer, layer::Context, prelude::*, registry::LookupSpan, util::SubscriberInitExt,
};

const WORKER_LOG_TARGET: &str = "worker_log";

#[derive(Debug)]
struct WorkerLogRecord {
    run_id: Option<i32>,
    node_id: Option<String>,
    worker_id: String,
    role: String,
    level: String,
    event_type: String,
    message: String,
    fields: JsonValue,
}

#[derive(Debug, Clone)]
struct DbLogLayer {
    tx: Arc<mpsc::UnboundedSender<WorkerLogRecord>>,
}

impl DbLogLayer {
    fn new(tx: mpsc::UnboundedSender<WorkerLogRecord>) -> Self {
        Self { tx: Arc::new(tx) }
    }
}

impl<S> Layer<S> for DbLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata.target() != WORKER_LOG_TARGET {
            return;
        }

        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        let mut fields = visitor.fields;

        let run_id = remove_i32(&mut fields, "run_id");
        let node_id = remove_string(&mut fields, "node_id");
        let worker_id = remove_string(&mut fields, "worker_id").unwrap_or_default();
        let role = remove_string(&mut fields, "role").unwrap_or_else(|| "unknown".to_string());
        let event_type =
            remove_string(&mut fields, "event_type").unwrap_or_else(|| metadata.name().to_string());
        let message = remove_string(&mut fields, "message").unwrap_or_else(|| event_type.clone());
        let level = metadata.level().to_string().to_lowercase();

        if worker_id.is_empty() {
            return;
        }

        let record = WorkerLogRecord {
            run_id,
            node_id,
            worker_id,
            role,
            level,
            event_type,
            message,
            fields: JsonValue::Object(fields),
        };

        let _ = self.tx.send(record);
    }
}

#[derive(Default)]
struct JsonVisitor {
    fields: JsonMap<String, JsonValue>,
}

impl tracing::field::Visit for JsonVisitor {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), JsonValue::Bool(value));
    }

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

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        let json_value = JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null);
        self.fields.insert(field.name().to_string(), json_value);
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

async fn write_worker_logs(pool: PgPool, mut rx: mpsc::UnboundedReceiver<WorkerLogRecord>) {
    while let Some(record) = rx.recv().await {
        let result = sqlx::query(
            r#"
            INSERT INTO worker_logs (
                run_id,
                node_id,
                worker_id,
                role,
                level,
                event_type,
                message,
                fields
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(record.run_id)
        .bind(record.node_id)
        .bind(record.worker_id)
        .bind(record.role)
        .bind(record.level)
        .bind(record.event_type)
        .bind(record.message)
        .bind(record.fields)
        .execute(&pool)
        .await;

        if let Err(err) = result {
            tracing::warn!(target: "telemetry", error = %err, "failed to persist worker log event");
        }
    }
}

/// Initialize global tracing subscriber.
///
/// If `worker_log_pool` is provided, events with target `worker_log` are also
/// persisted into the `worker_logs` table.
pub fn init_tracing(worker_log_pool: Option<PgPool>) -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    if let Some(pool) = worker_log_pool {
        let (tx, rx) = mpsc::unbounded_channel::<WorkerLogRecord>();
        tokio::spawn(write_worker_logs(pool, rx));

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(DbLogLayer::new(tx))
            .try_init()?;
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init()?;
    }

    Ok(())
}
