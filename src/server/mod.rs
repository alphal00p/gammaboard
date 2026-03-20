mod config_panels;
mod panels;
mod performance_panels;
mod run_panels;
mod task_panels;
mod worker_panels;

use crate::core::{AggregationStore, RunReadStore, RunSpecStore, RunTaskStore, StoreError};
use crate::evaluation::ObservableState;
use crate::server::config_panels::{
    EvaluatorPanelContext, PanelRenderer, SamplerAggregatorPanelContext,
};
use crate::server::panels::{PanelRequest, PanelResponse};
use crate::server::performance_panels::{
    build_evaluator_performance_response, build_sampler_performance_response,
};
use crate::server::run_panels::build_run_panel_response;
use crate::server::task_panels::{TaskPanelSource, parse_cursor as parse_task_panel_cursor};
use crate::server::worker_panels::build_worker_panel_response;
use crate::stores::PgStore;
use anyhow::Context;
use axum::{
    Router,
    extract::{Json as AxumJson, Path as AxumPath, Query, State},
    http::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde::Serialize;
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
struct TaskPanelRequest {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(flatten)]
    request: PanelRequest,
}

fn default_limit() -> i64 {
    1000
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_log_limit")]
    limit: i64,
    node_name: Option<String>,
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
    node_name: Option<String>,
}

fn default_perf_history_limit() -> i64 {
    500
}

fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, 10_000)
}

fn json_response<T: Serialize>(value: T) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(value).map_err(|err| ApiError::Internal(err.to_string()))?,
    ))
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
        .route("/nodes/:id/panels", get(get_node_panels))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/panels", get(get_run_panels))
        .route("/runs/:id/tasks", get(get_run_tasks))
        .route("/runs/:id/config/evaluator", get(get_run_evaluator_config))
        .route(
            "/runs/:id/config/sampler-aggregator",
            get(get_run_sampler_aggregator_config),
        )
        .route("/runs/:id/tasks/:task_id/output", post(get_run_task_output))
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
    json_response(runs)
}

async fn get_nodes(
    State(state): State<AppState>,
    Query(params): Query<WorkersQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let workers = state.store.get_registered_workers(params.run_id).await?;
    json_response(workers)
}

async fn get_node_panels(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let worker = state
        .store
        .get_registered_workers(None)
        .await?
        .into_iter()
        .find(|worker| worker.node_name == node_name)
        .ok_or_else(|| ApiError::NotFound(format!("node {node_name} not found")))?;
    json_response(build_worker_panel_response(&worker))
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
    json_response(run)
}

async fn get_run_panels(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let tasks = state.store.list_run_tasks(run_id).await?;
    let workers = state.store.get_registered_workers(Some(run_id)).await?;
    json_response(
        build_run_panel_response(&run, &run_spec, &tasks, &workers)
            .map_err(|err| ApiError::Internal(err.to_string()))?,
    )
}

async fn get_run_tasks(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let tasks = state.store.list_run_tasks(id).await?;
    json_response(tasks)
}

async fn get_run_evaluator_config(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let response: PanelResponse = run_spec
        .evaluator
        .build_response(
            format!("run:{run_id}:config:evaluator"),
            &EvaluatorPanelContext {
                point_spec: &run_spec.point_spec,
                runner_params: &run_spec.evaluator_runner_params,
                init_metadata: run.evaluator_init_metadata.as_ref(),
            },
        )
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    json_response(response)
}

async fn get_run_sampler_aggregator_config(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let response: PanelResponse = run_spec
        .sampler_aggregator
        .build_response(
            format!("run:{run_id}:config:sampler_aggregator"),
            &SamplerAggregatorPanelContext {
                point_spec: &run_spec.point_spec,
                runner_params: &run_spec.sampler_aggregator_runner_params,
            },
        )
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    json_response(response)
}

async fn get_run_task_output(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
    AxumJson(request): AxumJson<TaskPanelRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(request.limit);
    let cursor =
        parse_task_panel_cursor(request.request.cursor.as_deref()).map_err(ApiError::BadRequest)?;
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let panel_source = TaskPanelSource::new(&task.task, &run_spec);
    let latest_persisted_snapshot = state
        .store
        .get_task_output_snapshots(run_id, task.id, None, 1)
        .await?
        .into_iter()
        .next();
    let latest_stage_snapshot = state
        .store
        .get_latest_task_stage_snapshot(run_id, task.id)
        .await?;
    let current_observable = if matches!(task.state, crate::core::RunTaskState::Active) {
        state
            .store
            .load_current_observable(run_id)
            .await?
            .map(|current_observable| {
                ObservableState::from_json(&current_observable)
                    .map_err(|err| ApiError::Internal(err.to_string()))
            })
            .transpose()?
    } else {
        None
    };
    let delta_history_snapshots = if panel_source.needs_history() {
        state
            .store
            .get_task_output_snapshots(run_id, task.id, cursor.snapshot_id, limit)
            .await?
    } else {
        Vec::new()
    };
    let full_history_snapshots = if panel_source.needs_history() {
        state
            .store
            .get_task_output_snapshots(run_id, task.id, None, i64::MAX)
            .await?
    } else {
        Vec::new()
    };
    let payload = panel_source
        .build_response(
            format!("run:{run_id}:task:{}", task.id),
            cursor,
            &task,
            &run_spec,
            &request.request.panel_state,
            current_observable.as_ref(),
            latest_stage_snapshot.as_ref(),
            latest_persisted_snapshot.as_ref(),
            &full_history_snapshots,
            &delta_history_snapshots,
        )
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    json_response(payload)
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
    json_response(stats)
}

async fn get_run_logs(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<LogQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let logs = state
        .store
        .get_worker_logs(
            id,
            limit,
            params.node_name.as_deref(),
            params.level.as_deref(),
            params
                .q
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            params.before_id,
        )
        .await?;
    json_response(logs)
}

async fn get_run_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let scope_id = params.node_name.clone().unwrap_or_else(|| id.to_string());
    let rows = state
        .store
        .get_evaluator_performance_history(id, limit, params.node_name.as_deref())
        .await?;
    json_response(build_evaluator_performance_response(Some(scope_id), rows))
}

async fn get_run_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let scope_id = params.node_name.clone().unwrap_or_else(|| id.to_string());
    let rows = state
        .store
        .get_sampler_performance_history(id, limit, params.node_name.as_deref())
        .await?;
    json_response(build_sampler_performance_response(Some(scope_id), rows))
}

async fn get_node_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let payload = state
        .store
        .get_worker_evaluator_performance_history(&node_name, limit)
        .await?;
    json_response(build_evaluator_performance_response(
        Some(node_name),
        payload,
    ))
}

async fn get_node_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let payload = state
        .store
        .get_worker_sampler_performance_history(&node_name, limit)
        .await?;
    json_response(build_sampler_performance_response(Some(node_name), payload))
}
