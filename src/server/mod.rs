mod panels;
mod performance_panels;
mod task_panels;

use crate::core::{AggregationStore, RunReadStore, RunSpecStore, RunTaskStore, StoreError};
use crate::evaluation::ObservableState;
use crate::server::panels::{TaskHistoryResponse, TaskOutputResponse};
use crate::server::performance_panels::{
    build_evaluator_performance_response, build_sampler_performance_response,
};
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
    after_snapshot_id: Option<i64>,
}

fn default_limit() -> i64 {
    1000
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_log_limit")]
    limit: i64,
    node_id: Option<String>,
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
    node_id: Option<String>,
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

fn build_app(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/runs", get(get_runs))
        .route("/nodes", get(get_nodes))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/tasks", get(get_run_tasks))
        .route("/runs/:id/tasks/:task_id/output", get(get_run_task_output))
        .route(
            "/runs/:id/tasks/:task_id/output/history",
            get(get_run_task_output_history),
        )
        .route("/runs/:id/stats", get(get_run_stats))
        .route("/runs/:id/logs", get(get_run_logs))
        .route(
            "/runs/:id/performance/evaluator",
            get(get_run_evaluator_performance_history),
        )
        .route(
            "/runs/:id/performance/sampler-aggregator",
            get(get_run_sampler_performance_history),
        )
        .route(
            "/nodes/:id/performance/evaluator",
            get(get_node_evaluator_performance_history),
        )
        .route(
            "/nodes/:id/performance/sampler-aggregator",
            get(get_node_sampler_performance_history),
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

async fn get_nodes(
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

async fn get_run_tasks(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let tasks = state.store.list_run_tasks(id).await?;
    Ok(Json(
        serde_json::to_value(tasks).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_task_output(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let latest_persisted_snapshot = state
        .store
        .get_task_output_snapshots(run_id, task.id, None, 1)
        .await?
        .into_iter()
        .next();
    let latest_snapshot_id = latest_persisted_snapshot
        .as_ref()
        .map(|snapshot| snapshot.id.clone());
    let latest_stage_snapshot = state
        .store
        .get_latest_task_stage_snapshot(run_id, task.id)
        .await?;
    let current_output = if matches!(task.state, crate::core::RunTaskState::Active) {
        match state.store.load_current_observable(run_id).await? {
            Some(current_observable) => {
                let observable = ObservableState::from_json(&current_observable)
                    .map_err(|err| ApiError::Internal(err.to_string()))?;
                task.task
                    .build_current_panels(&task, Some(&observable), &run_spec)
                    .map_err(|err| ApiError::Internal(err.to_string()))?
            }
            None => match latest_persisted_snapshot.as_ref() {
                Some(snapshot) => task
                    .task
                    .build_current_panels_from_persisted(
                        &task,
                        &snapshot.persisted_output,
                        &run_spec,
                    )
                    .map_err(|err| ApiError::Internal(err.to_string()))?,
                None => task
                    .task
                    .build_current_panels(&task, None, &run_spec)
                    .map_err(|err| ApiError::Internal(err.to_string()))?,
            },
        }
    } else {
        match latest_stage_snapshot.as_ref() {
            Some(snapshot) => task
                .task
                .build_current_panels_from_stage_snapshot(&task, snapshot, &run_spec)
                .map_err(|err| ApiError::Internal(err.to_string()))?,
            None => match latest_persisted_snapshot.as_ref() {
                Some(snapshot) => task
                    .task
                    .build_current_panels_from_persisted(
                        &task,
                        &snapshot.persisted_output,
                        &run_spec,
                    )
                    .map_err(|err| ApiError::Internal(err.to_string()))?,
                None => task
                    .task
                    .build_current_panels(&task, None, &run_spec)
                    .map_err(|err| ApiError::Internal(err.to_string()))?,
            },
        }
    };

    let payload = TaskOutputResponse {
        task_id: task.id.to_string(),
        sequence_nr: task.sequence_nr,
        task_kind: task.task.kind_str().to_string(),
        task_state: task.state.as_str().to_string(),
        panels: task.task.describe_panels(&run_spec),
        current: current_output,
        latest_snapshot_id,
        updated_at: task
            .completed_at
            .or(task.failed_at)
            .or(task.started_at)
            .or(Some(task.created_at)),
    };

    Ok(Json(
        serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_task_output_history(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
    Query(params): Query<LimitQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let snapshots = state
        .store
        .get_task_output_snapshots(run_id, task.id, params.after_snapshot_id, limit)
        .await?;
    let latest_snapshot_id = snapshots
        .first()
        .map(|item| item.id.clone())
        .or_else(|| params.after_snapshot_id.map(|id| id.to_string()));
    let snapshots = snapshots
        .into_iter()
        .rev()
        .map(|snapshot| task.task.build_history_item(&task, &snapshot, &run_spec))
        .map(|item| item.map_err(|err| ApiError::Internal(err.to_string())))
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(
        serde_json::to_value(TaskHistoryResponse {
            task_id: task.id.to_string(),
            latest_snapshot_id,
            reset_required: false,
            items: snapshots,
        })
        .map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn load_run_task(
    store: &PgStore,
    run_id: i32,
    task_id: i64,
) -> Result<crate::core::RunTask, ApiError> {
    store
        .list_run_tasks(run_id)
        .await?
        .into_iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| ApiError::NotFound(format!("task {task_id} not found for run {run_id}")))
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
            params.node_id.as_deref(),
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

async fn get_run_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let scope_id = params.node_id.clone().unwrap_or_else(|| id.to_string());
    let rows = state
        .store
        .get_evaluator_performance_history(id, limit, params.node_id.as_deref())
        .await?;
    Ok(Json(
        serde_json::to_value(build_evaluator_performance_response(Some(scope_id), rows))
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_run_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let scope_id = params.node_id.clone().unwrap_or_else(|| id.to_string());
    let rows = state
        .store
        .get_sampler_performance_history(id, limit, params.node_id.as_deref())
        .await?;
    Ok(Json(
        serde_json::to_value(build_sampler_performance_response(Some(scope_id), rows))
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_node_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(node_id): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let payload = state
        .store
        .get_worker_evaluator_performance_history(&node_id, limit)
        .await?;
    Ok(Json(
        serde_json::to_value(build_evaluator_performance_response(Some(node_id), payload))
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

async fn get_node_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(node_id): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.clamp(1, 10_000);
    let payload = state
        .store
        .get_worker_sampler_performance_history(&node_id, limit)
        .await?;
    Ok(Json(
        serde_json::to_value(build_sampler_performance_response(Some(node_id), payload))
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}
