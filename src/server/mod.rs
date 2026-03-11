use crate::core::{RunReadStore, StoreError};
use crate::stores::PgStore;
use anyhow::Context;
use axum::{
    Router,
    extract::{Path as AxumPath, Query, State},
    http::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::get,
};
use serde::Deserialize;
use std::{env, net::SocketAddr};
use tower_http::cors::CorsLayer;
use tracing::Instrument;

pub fn resolve_bind(bind: Option<SocketAddr>) -> anyhow::Result<SocketAddr> {
    match bind {
        Some(bind) => Ok(bind),
        None => {
            let value = env::var("GAMMABOARD_BACKEND_PORT").context(
                "missing GAMMABOARD_BACKEND_PORT (set it in environment or pass --bind)",
            )?;
            let port = value
                .parse::<u16>()
                .with_context(|| format!("invalid GAMMABOARD_BACKEND_PORT={value:?}"))?;
            Ok(SocketAddr::from(([0, 0, 0, 0], port)))
        }
    }
}

pub async fn serve(store: PgStore, bind: SocketAddr) -> anyhow::Result<()> {
    let state = AppState { store };

    let app = build_app(state);

    tracing::info!("server listening on http://{}", bind);
    tracing::info!("api available at http://{}/api", bind);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind server socket at {bind}"))?;
    axum::serve(listener, app)
        .await
        .context("api server exited with error")?;

    Ok(())
}

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
    q: Option<String>,
    before_id: Option<i64>,
}

fn default_log_limit() -> i64 {
    500
}

#[derive(Deserialize)]
struct PerformanceHistoryQuery {
    #[serde(default = "default_perf_history_limit")]
    limit: i64,
    worker_id: Option<String>,
}

fn default_perf_history_limit() -> i64 {
    500
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::InvalidInput(message) => Self::BadRequest(message),
            StoreError::NotFound(message) => Self::NotFound(message),
            StoreError::Internal(message)
            | StoreError::Database(message)
            | StoreError::Serialization(message) => Self::Internal(message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[derive(Deserialize)]
struct WorkersQuery {
    run_id: Option<i32>,
}

#[derive(Deserialize)]
struct AggregatedRangeQuery {
    start: i64,
    stop: i64,
    max_points: i64,
    last_id: Option<i64>,
}

fn build_app(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/runs", get(get_runs))
        .route("/workers", get(get_workers))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/stats", get(get_run_stats))
        .route("/runs/:id/logs", get(get_run_logs))
        .route("/runs/:id/aggregated", get(get_run_aggregated_results))
        .route("/runs/:id/aggregated/range", get(get_run_aggregated_range))
        .route(
            "/runs/:id/aggregated/latest",
            get(get_run_aggregated_latest),
        )
        .route(
            "/runs/:id/performance/evaluator",
            get(get_run_evaluator_performance_history),
        )
        .route(
            "/runs/:id/performance/sampler-aggregator",
            get(get_run_sampler_performance_history),
        )
        .route(
            "/workers/:id/performance/evaluator",
            get(get_worker_evaluator_performance_history),
        )
        .route(
            "/workers/:id/performance/sampler-aggregator",
            get(get_worker_sampler_performance_history),
        )
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state);

    Router::new()
        .nest("/api", api_routes)
        .layer(CorsLayer::permissive())
}

async fn request_context_middleware(request: Request<axum::body::Body>, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "api_request",
        source = "server",
        method = %method,
        path = %path
    );
    next.run(request).instrument(span).await
}

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

async fn get_runs(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let runs = state.store.get_all_runs().await?;
    Ok(Json(
        serde_json::to_value(runs).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_workers(
    State(state): State<AppState>,
    Query(params): Query<WorkersQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let workers = state.store.get_registered_workers(params.run_id).await?;
    Ok(Json(
        serde_json::to_value(workers).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Run not found".to_string()))?;
    Ok(Json(
        serde_json::to_value(run).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_stats(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let stats = state.store.get_work_queue_stats(id).await?;
    Ok(Json(
        serde_json::to_value(stats).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_logs(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<LogQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let logs = state
        .store
        .get_worker_logs(
            id,
            limit,
            params.worker_id.as_deref(),
            params.level.as_deref(),
            params
                .q
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            params.before_id,
        )
        .await?;
    Ok(Json(
        serde_json::to_value(logs).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_aggregated_results(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<LimitQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let results = state.store.get_aggregated_results(id, limit).await?;
    Ok(Json(
        serde_json::to_value(results).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_aggregated_range(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<AggregatedRangeQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    if params.max_points < 1 {
        return Err(ApiError::BadRequest("max_points must be >= 1".to_string()));
    }

    let result = state
        .store
        .get_aggregated_range(
            id,
            params.start,
            params.stop,
            params.max_points,
            params.last_id,
        )
        .await?;
    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_aggregated_latest(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let result = state
        .store
        .get_latest_aggregated_result(id)
        .await?
        .ok_or_else(|| ApiError::NotFound("No aggregated results".to_string()))?;
    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let rows = state
        .store
        .get_evaluator_performance_history(id, limit, params.worker_id.as_deref())
        .await?;
    Ok(Json(
        serde_json::to_value(rows).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let rows = state
        .store
        .get_sampler_performance_history(id, limit, params.worker_id.as_deref())
        .await?;
    Ok(Json(
        serde_json::to_value(rows).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_worker_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(worker_id): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let payload = state
        .store
        .get_worker_evaluator_performance_history(&worker_id, limit)
        .await?;
    Ok(Json(
        serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_worker_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(worker_id): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let payload = state
        .store
        .get_worker_sampler_performance_history(&worker_id, limit)
        .await?;
    Ok(Json(
        serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}
