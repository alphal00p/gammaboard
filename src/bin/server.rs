//! Gammaboard API Server
//!
//! Serves the REST API for the dashboard and static frontend files.

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
};
use futures_core::Stream;
use gammaboard::stores::RunReadStore;
use gammaboard::{BinResult, PgStore, init_pg_store};
use serde::Deserialize;
use std::{
    convert::Infallible,
    fmt::Display,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    store: PgStore,
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    1000
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_log_limit")]
    limit: i64,
    worker_id: Option<String>,
    level: Option<String>,
}

fn default_log_limit() -> i64 {
    500
}

fn default_stream_interval_ms() -> u64 {
    1000
}

fn sanitize_stream_interval_ms(interval_ms: u64) -> u64 {
    const MIN_INTERVAL_MS: u64 = 100;
    const MAX_INTERVAL_MS: u64 = 60_000;
    interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn internal_error(context: impl Display, err: impl Display, message: &str) -> Response {
    eprintln!("Error {}: {}", context, err);
    json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(default = "default_stream_interval_ms")]
    interval_ms: u64,
}

struct MpscStream {
    receiver: mpsc::Receiver<Result<Event, Infallible>>,
}

impl Stream for MpscStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.receiver).poll_recv(cx)
    }
}

async fn send_sse_error(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    message: &str,
    details: impl Display,
) -> bool {
    let err_event = Event::default().event("error").data(
        serde_json::json!({
            "error": message,
            "details": details.to_string()
        })
        .to_string(),
    );
    tx.send(Ok(err_event)).await.is_ok()
}

#[tokio::main]
async fn main() -> BinResult {
    println!("🚀 Starting Gammaboard API Server...");

    // Initialize database connection pool
    let store = init_pg_store(10).await?;
    println!("✅ Connected to database");

    let state = AppState { store };

    // Build API routes
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/runs", get(get_runs))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/stats", get(get_run_stats))
        .route("/runs/:id/logs", get(get_run_logs))
        .route("/runs/:id/aggregated", get(get_run_aggregated_results))
        .route(
            "/runs/:id/aggregated/latest",
            get(get_run_aggregated_latest),
        )
        .route("/runs/:id/stream", get(stream_run_stats))
        .with_state(state);

    // Build main app with CORS and static file serving
    let app = Router::new()
        .nest("/api", api_routes)
        // Serve static files from dashboard build directory
        // Uncomment when you have a production build:
        // .nest_service("/", ServeDir::new("dashboard/frontend/build"))
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([0, 0, 0, 0], 4000));
    println!("🌐 Server listening on http://{}", addr);
    println!("📊 API available at http://{}/api", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ============================================================================
// API Handlers
// ============================================================================

/// Health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.health_check().await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "database": "connected"
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "database": "disconnected"
            })),
        )
            .into_response(),
    }
}

/// Get all runs with progress
async fn get_runs(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.get_all_runs().await {
        Ok(runs) => (StatusCode::OK, Json(runs)).into_response(),
        Err(e) => internal_error("fetching runs", e, "Failed to fetch runs"),
    }
}

/// Get specific run progress
async fn get_run(State(state): State<AppState>, Path(id): Path<i32>) -> impl IntoResponse {
    match state.store.get_run_progress(id).await {
        Ok(Some(run)) => (StatusCode::OK, Json(run)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "Run not found"),
        Err(e) => internal_error(format!("fetching run {id}"), e, "Failed to fetch run"),
    }
}

/// Get work queue statistics for a run
async fn get_run_stats(State(state): State<AppState>, Path(id): Path<i32>) -> impl IntoResponse {
    match state.store.get_work_queue_stats(id).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(e) => internal_error(
            format!("fetching stats for run {id}"),
            e,
            "Failed to fetch stats",
        ),
    }
}

/// Get worker logs for a run.
async fn get_run_logs(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<LogQuery>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 10_000);
    match state
        .store
        .get_worker_logs(
            id,
            limit,
            params.worker_id.as_deref(),
            params.level.as_deref(),
        )
        .await
    {
        Ok(logs) => (StatusCode::OK, Json(logs)).into_response(),
        Err(e) => internal_error(
            format!("fetching logs for run {id}"),
            e,
            "Failed to fetch logs",
        ),
    }
}

/// Get aggregated results history for a run
async fn get_run_aggregated_results(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    match state.store.get_aggregated_results(id, params.limit).await {
        Ok(results) => (StatusCode::OK, Json(results)).into_response(),
        Err(e) => internal_error(
            format!("fetching aggregated results for run {id}"),
            e,
            "Failed to fetch aggregated results",
        ),
    }
}

/// Get latest aggregated results for a run
async fn get_run_aggregated_latest(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match state.store.get_latest_aggregated_result(id).await {
        Ok(Some(result)) => (StatusCode::OK, Json(result)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "No aggregated results"),
        Err(e) => internal_error(
            format!("fetching latest aggregated result for run {id}"),
            e,
            "Failed to fetch aggregated results",
        ),
    }
}

/// Live stats stream (SSE)
async fn stream_run_stats(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<StreamQuery>,
) -> impl IntoResponse {
    match state.store.get_run_progress(id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return json_error(StatusCode::NOT_FOUND, "Run not found");
        }
        Err(e) => {
            return internal_error(
                format!("validating run {id} for stream"),
                e,
                "Failed to initialize stream",
            );
        }
    }

    let (tx, rx) = mpsc::channel(16);
    let store = state.store.clone();
    let interval_ms = sanitize_stream_interval_ms(params.interval_ms);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;

            let progress = match store.get_run_progress(id).await {
                Ok(run) => run,
                Err(e) => {
                    if !send_sse_error(&tx, "Failed to fetch run progress", e).await {
                        break;
                    }
                    continue;
                }
            };

            let aggregated = match store.get_latest_aggregated_result(id).await {
                Ok(result) => result,
                Err(e) => {
                    if !send_sse_error(&tx, "Failed to fetch aggregated results", e).await {
                        break;
                    }
                    continue;
                }
            };

            let payload = serde_json::json!({
                "run": progress,
                "aggregated": aggregated
            });

            let event = Event::default().event("stats").data(payload.to_string());

            if tx.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    Sse::new(MpscStream { receiver: rx })
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("keep-alive"),
        )
        .into_response()
}
