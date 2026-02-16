//! Gammaboard API Server
//!
//! Serves the REST API for the dashboard and static frontend files.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use gammaboard::{get_pg_pool, DbPool};
use serde::Deserialize;
use std::net::SocketAddr;
use tower_http::{
    cors::CorsLayer,
    services::ServeDir,
};

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
        .route("/runs/:id/batches", get(get_run_batches))
        .route("/runs/:id/samples", get(get_run_samples))
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
        Ok(_) => Json(serde_json::json!({
            "status": "ok",
            "database": "connected"
        })),
        Err(_) => Json(serde_json::json!({
            "status": "error",
            "database": "disconnected"
        })),
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
async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
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
async fn get_run_stats(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
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

/// Get completed batches for a run
async fn get_run_batches(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    match gammaboard::get_completed_batches(&state.pool, id, params.limit).await {
        Ok(batches) => (StatusCode::OK, Json(batches)).into_response(),
        Err(e) => {
            eprintln!("Error fetching batches for run {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch batches" })),
            )
                .into_response()
        }
    }
}

/// Get flattened sample data for visualization
async fn get_run_samples(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    match gammaboard::get_sample_data(&state.pool, id, params.limit).await {
        Ok(samples) => (StatusCode::OK, Json(samples)).into_response(),
        Err(e) => {
            eprintln!("Error fetching samples for run {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to fetch samples" })),
            )
                .into_response()
        }
    }
}
