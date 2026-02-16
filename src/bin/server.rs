//! Gammaboard API Server
//!
//! Serves the REST API for the dashboard and static frontend files.

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use futures_core::Stream;
use gammaboard::{DbPool, get_pg_pool};
use serde::Deserialize;
use std::{
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    pool: DbPool,
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    1000
}

fn default_stream_interval_ms() -> u64 {
    1000
}

fn sanitize_stream_interval_ms(interval_ms: u64) -> u64 {
    const MIN_INTERVAL_MS: u64 = 100;
    const MAX_INTERVAL_MS: u64 = 60_000;
    interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Gammaboard API Server...");

    // Initialize database connection pool
    let pool = get_pg_pool(10).await?;
    println!("✅ Connected to database");

    let state = AppState { pool };

    // Build API routes
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/runs", get(get_runs))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/stats", get(get_run_stats))
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
    match sqlx::query("SELECT 1").fetch_one(&state.pool).await {
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
    match gammaboard::get_all_runs(&state.pool).await {
        Ok(runs) => (StatusCode::OK, Json(runs)).into_response(),
        Err(e) => {
            eprintln!("Error fetching runs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch runs" })),
            )
                .into_response()
        }
    }
}

/// Get specific run progress
async fn get_run(State(state): State<AppState>, Path(id): Path<i32>) -> impl IntoResponse {
    match gammaboard::get_run_progress(&state.pool, id).await {
        Ok(Some(run)) => (StatusCode::OK, Json(run)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Run not found" })),
        )
            .into_response(),
        Err(e) => {
            eprintln!("Error fetching run {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch run" })),
            )
                .into_response()
        }
    }
}

/// Get work queue statistics for a run
async fn get_run_stats(State(state): State<AppState>, Path(id): Path<i32>) -> impl IntoResponse {
    match gammaboard::get_work_queue_stats(&state.pool, id).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(e) => {
            eprintln!("Error fetching stats for run {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch stats" })),
            )
                .into_response()
        }
    }
}

/// Get aggregated results history for a run
async fn get_run_aggregated_results(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    match gammaboard::get_aggregated_results(&state.pool, id, params.limit).await {
        Ok(results) => (StatusCode::OK, Json(results)).into_response(),
        Err(e) => {
            eprintln!("Error fetching aggregated results for run {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch aggregated results" })),
            )
                .into_response()
        }
    }
}

/// Get latest aggregated results for a run
async fn get_run_aggregated_latest(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match gammaboard::get_latest_aggregated_result(&state.pool, id).await {
        Ok(Some(result)) => (StatusCode::OK, Json(result)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No aggregated results" })),
        )
            .into_response(),
        Err(e) => {
            eprintln!(
                "Error fetching latest aggregated result for run {}: {}",
                id, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch aggregated results" })),
            )
                .into_response()
        }
    }
}

/// Live stats stream (SSE)
async fn stream_run_stats(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<StreamQuery>,
) -> impl IntoResponse {
    match gammaboard::get_run_progress(&state.pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Run not found" })),
            )
                .into_response();
        }
        Err(e) => {
            eprintln!("Error validating run {} for stream: {}", id, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to initialize stream" })),
            )
                .into_response();
        }
    }

    let (tx, rx) = mpsc::channel(16);
    let pool = state.pool.clone();
    let interval_ms = sanitize_stream_interval_ms(params.interval_ms);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;

            let progress = match gammaboard::get_run_progress(&pool, id).await {
                Ok(run) => run,
                Err(e) => {
                    let err_event = Event::default().event("error").data(
                        serde_json::json!({
                            "error": "Failed to fetch run progress",
                            "details": e.to_string()
                        })
                        .to_string(),
                    );
                    if tx.send(Ok(err_event)).await.is_err() {
                        break;
                    }
                    continue;
                }
            };

            let aggregated = match gammaboard::get_latest_aggregated_result(&pool, id).await {
                Ok(result) => result,
                Err(e) => {
                    let err_event = Event::default().event("error").data(
                        serde_json::json!({
                            "error": "Failed to fetch aggregated results",
                            "details": e.to_string()
                        })
                        .to_string(),
                    );
                    if tx.send(Ok(err_event)).await.is_err() {
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
