mod auth;
mod config_panels;
mod panels;
mod performance_panels;
mod run_panels;
mod task_panels;
mod worker_panels;

use crate::api::{ApiError, nodes as node_api, runs as run_api, templates as template_api};
use crate::core::{AggregationStore, RunReadStore, RunSpecStore, RunTask, RunTaskStore};
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
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde::Serialize;
use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};
use tower_http::cors::CorsLayer;
use tracing::Instrument;

use self::auth::{AuthConfig, SessionStatus, login, logout, require_admin_session};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
    pub allowed_origin: String,
    pub secure_cookie: bool,
    pub run_templates_dir: String,
    pub task_templates_dir: String,
    pub auth: ServerAuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerAuthConfig {
    pub admin_password_hash: String,
    pub session_secret: String,
}

impl ServerConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed reading server config {}", path.display()))?;
        let mut parsed: Self = toml::from_str(&raw)
            .with_context(|| format!("failed parsing server config {}", path.display()))?;
        let base_dir = path
            .parent()
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")));
        parsed.run_templates_dir =
            normalize_config_path(base_dir, parsed.run_templates_dir.as_str())
                .display()
                .to_string();
        parsed.task_templates_dir =
            normalize_config_path(base_dir, parsed.task_templates_dir.as_str())
                .display()
                .to_string();
        Ok(parsed)
    }

    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

pub async fn serve(store: PgStore, config: ServerConfig) -> anyhow::Result<()> {
    let bind = config.bind_addr();
    let allowed_origin = axum::http::HeaderValue::from_str(config.allowed_origin.trim())
        .with_context(|| format!("invalid server.allowed_origin={:?}", config.allowed_origin))?;
    let state = AppState {
        store,
        auth: AuthConfig::from_server_config(&config.auth),
        allowed_origin,
        secure_cookie: config.secure_cookie,
        run_templates_dir: PathBuf::from(&config.run_templates_dir),
        task_templates_dir: PathBuf::from(&config.task_templates_dir),
    };

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
pub(crate) struct AppState {
    store: PgStore,
    auth: AuthConfig,
    allowed_origin: axum::http::HeaderValue,
    secure_cookie: bool,
    run_templates_dir: PathBuf,
    task_templates_dir: PathBuf,
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
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
struct AssignNodeRequest {
    run_id: i32,
    role: String,
}

#[derive(Deserialize)]
struct AutoAssignRequest {
    max_evaluators: Option<usize>,
}

#[derive(Deserialize)]
struct CreateRunRequest {
    toml: String,
}

#[derive(Deserialize)]
struct CloneRunRequest {
    source_run_id: i32,
    #[serde(
        deserialize_with = "crate::utils::serde_bigint::deserialize_i64_from_string_or_number"
    )]
    from_snapshot_id: i64,
    new_name: String,
}

#[derive(Deserialize)]
struct AddTasksRequest {
    toml: String,
}

#[derive(Serialize)]
struct TemplateListResponse {
    items: Vec<String>,
}

#[derive(Serialize)]
struct TemplateFileResponse {
    name: String,
    toml: String,
}

#[derive(Serialize)]
struct RunTaskResponse {
    #[serde(flatten)]
    task: RunTask,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    latest_stage_snapshot_id: Option<i64>,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    root_stage_snapshot_id: Option<i64>,
}

fn build_app(state: AppState) -> Router {
    let public_api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/auth/session", get(get_session_status))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/runs", get(get_runs))
        .route("/nodes", get(get_nodes))
        .route("/nodes/:id/panels", get(get_node_panels))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/panels", get(get_run_panels))
        .route("/runs/:id/tasks", get(get_run_tasks))
        .route("/templates/runs", get(list_run_templates))
        .route("/templates/runs/:name", get(get_run_template))
        .route("/templates/tasks", get(list_task_templates))
        .route("/templates/tasks/:name", get(get_task_template))
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
        );

    let protected_api_routes = Router::new()
        .route("/runs", post(create_run))
        .route("/runs/clone", post(clone_run))
        .route("/runs/:id", delete(delete_run))
        .route("/runs/:id/pause", post(pause_run))
        .route("/runs/:id/tasks", post(add_run_tasks))
        .route("/runs/:id/tasks/:task_id", delete(delete_run_task))
        .route("/runs/:id/auto-assign", post(auto_assign_run))
        .route("/nodes/:id/assign", post(assign_node))
        .route("/nodes/:id/unassign", post(unassign_node))
        .route("/nodes/:id/stop", post(stop_node))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_session,
        ));

    Router::new()
        .nest("/api", public_api_routes.merge(protected_api_routes))
        .layer(build_cors_layer(state.allowed_origin.clone()))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state)
}

fn build_cors_layer(allowed_origin: axum::http::HeaderValue) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_origin(allowed_origin)
}

fn normalize_config_path(base_dir: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path.trim());
    if candidate.is_absolute() {
        candidate
    } else {
        base_dir.join(candidate)
    }
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

async fn get_session_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<SessionStatus> {
    Json(auth::auth_status_from_headers(&state, &headers))
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
    let latest_snapshot_ids = state
        .store
        .list_latest_stage_snapshot_ids_by_task(id)
        .await?;
    let root_stage_snapshot_id = state.store.get_root_stage_snapshot_id(id).await?;
    let response = tasks
        .into_iter()
        .map(|task| RunTaskResponse {
            latest_stage_snapshot_id: latest_snapshot_ids.get(&task.id).copied(),
            root_stage_snapshot_id,
            task,
        })
        .collect::<Vec<_>>();
    json_response(response)
}

async fn list_run_templates(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: template_api::list_templates(&state.run_templates_dir)?,
    })
}

async fn get_run_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template = template_api::load_template(&state.run_templates_dir, &name)?;
    json_response(TemplateFileResponse {
        name: template.name,
        toml: template.toml,
    })
}

async fn list_task_templates(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: template_api::list_templates(&state.task_templates_dir)?,
    })
}

async fn get_task_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template = template_api::load_template(&state.task_templates_dir, &name)?;
    json_response(TemplateFileResponse {
        name: template.name,
        toml: template.toml,
    })
}

async fn get_run_evaluator_config(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
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
            },
        )
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    json_response(response)
}

async fn get_run_sampler_aggregator_config(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let integration_params = run_api::decode_integration_params(
        run_id,
        run.integration_params.ok_or_else(|| {
            ApiError::Internal(format!("run {run_id} is missing integration_params"))
        })?,
    )?;
    let fallback_sampler_config = integration_params.sampler_aggregator.clone();
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let sampler_config = if let Some(task) = state.store.load_active_run_task(run_id).await? {
        resolve_active_task_sampler_config(&state, run_id, &task, &fallback_sampler_config).await?
    } else {
        fallback_sampler_config
    };
    let response: PanelResponse = sampler_config
        .build_response(
            format!("run:{run_id}:config:sampler_aggregator"),
            &SamplerAggregatorPanelContext {
                point_spec: &run_spec.point_spec,
                runner_params: &run_spec.sampler_aggregator_runner_params,
            },
        )
        .map_err(|err: crate::core::BuildError| ApiError::Internal(err.to_string()))?;
    json_response(response)
}

async fn resolve_active_task_sampler_config(
    state: &AppState,
    run_id: i32,
    task: &RunTask,
    fallback_sampler_config: &crate::core::SamplerAggregatorConfig,
) -> Result<crate::core::SamplerAggregatorConfig, ApiError> {
    if let Some(config) = task.task.sampler_config() {
        return Ok(config);
    }
    if let Some(config) = task.task.sample_sampler_config() {
        return Ok(config);
    }

    if let Some(source_snapshot) =
        resolve_task_source_snapshot(state, run_id, task, task.task.sample_sampler_source()).await?
    {
        return Ok(source_snapshot.sampler_aggregator);
    }

    if let Some(base_snapshot) = state
        .store
        .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
        .await?
    {
        return Ok(base_snapshot.sampler_aggregator);
    }

    Ok(fallback_sampler_config.clone())
}

async fn resolve_task_source_snapshot(
    state: &AppState,
    run_id: i32,
    task: &RunTask,
    source: Option<crate::core::SourceRefSpec>,
) -> Result<Option<crate::core::RunStageSnapshot>, ApiError> {
    match source {
        Some(crate::core::SourceRefSpec::Latest) => state
            .store
            .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
            .await
            .map_err(Into::into),
        Some(crate::core::SourceRefSpec::FromName(source_task_name)) => {
            let source_task = state
                .store
                .list_run_tasks(run_id)
                .await?
                .into_iter()
                .find(|candidate| candidate.name == source_task_name)
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "task {} references source task '{}' but no such task exists in run {}",
                        task.id, source_task_name, run_id
                    ))
                })?;
            if source_task.sequence_nr >= task.sequence_nr {
                return Err(ApiError::BadRequest(format!(
                    "task {} references source task '{}' which is not prior in sequence",
                    task.id, source_task_name
                )));
            }
            let snapshot = state
                .store
                .load_latest_stage_snapshot_before_sequence(run_id, source_task.sequence_nr + 1)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "task {} source task '{}' has no queue-empty stage snapshot",
                        task.id, source_task_name
                    ))
                })?;
            if snapshot.task_id != Some(source_task.id) {
                return Err(ApiError::BadRequest(format!(
                    "task {} source task '{}' has no queue-empty stage snapshot",
                    task.id, source_task_name
                )));
            }
            Ok(Some(snapshot))
        }
        None => Ok(None),
    }
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
    let panel_source = TaskPanelSource::new(&task.task);
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

async fn create_run(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<CreateRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = run_api::create_run(
        &state.store,
        run_api::parse_run_add_config_toml(payload.toml.trim())?,
    )
    .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_create",
        run_id = run.run_id,
        run_name = %run.run_name,
        tasks_created = run.tasks_created,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run.run_id,
        "run_name": run.run_name,
    }))
}

async fn clone_run(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<CloneRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = run_api::clone_run(
        &state.store,
        payload.source_run_id,
        payload.from_snapshot_id,
        &payload.new_name,
    )
    .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_clone",
        run_id = run.run_id,
        new_name = %run.run_name,
        source_run_id = run.source_run_id,
        from_snapshot_id = run.from_snapshot_id,
        cloned_tasks = run.cloned_tasks,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run.run_id,
        "run_name": run.run_name,
    }))
}

async fn add_run_tasks(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
    AxumJson(payload): AxumJson<AddTasksRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = run_api::append_tasks(
        &state.store,
        run_id,
        run_api::parse_task_queue_toml(payload.toml.trim())?,
    )
    .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_add_tasks",
        run_id,
        tasks_added = result.tasks.len(),
        "dashboard action completed"
    );
    json_response(result.tasks)
}

async fn pause_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = run_api::pause_run(&state.store, run_id).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_pause",
        run_id = result.run_id,
        run_name = %result.run_name,
        assignments_cleared = result.assignments_cleared,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": result.run_id,
        "assignments_cleared": result.assignments_cleared,
    }))
}

async fn delete_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = run_api::remove_run(&state.store, run_id).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_remove",
        run_id = result.run_id,
        run_name = %result.run_name,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": result.run_id,
        "run_name": result.run_name,
    }))
}

async fn delete_run_task(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = run_api::remove_pending_task(&state.store, run_id, task_id).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_task_remove",
        run_id = result.run_id,
        task_id = result.task_id,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": result.run_id,
        "task_id": result.task_id,
    }))
}

async fn assign_node(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
    AxumJson(payload): AxumJson<AssignNodeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let assigned = node_api::assign_node(
        &state.store,
        &node_name,
        payload.run_id,
        payload
            .role
            .parse()
            .map_err(|err: String| ApiError::BadRequest(err))?,
    )
    .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_assign",
        node_name = %assigned.node_name,
        run_id = assigned.run_id,
        run_name = %assigned.run_name,
        role = %assigned.role,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "node_name": assigned.node_name,
        "run_id": assigned.run_id,
        "role": assigned.role.as_str(),
    }))
}

async fn auto_assign_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
    AxumJson(payload): AxumJson<AutoAssignRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = node_api::auto_assign_run(&state.store, run_id, payload.max_evaluators).await?;

    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_auto_assign",
        run_id = result.run_id,
        run_name = %result.run_name,
        sampler_already_assigned = result.sampler_already_assigned,
        assigned_sampler = result.assigned_sampler.as_deref().unwrap_or("none"),
        assigned_evaluators = result.assigned_evaluators.len(),
        requested_evaluator_limit = payload.max_evaluators,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": result.run_id,
        "sampler_already_assigned": result.sampler_already_assigned,
        "assigned_sampler": result.assigned_sampler,
        "assigned_evaluators": result.assigned_evaluators,
    }))
}

async fn unassign_node(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    node_api::unassign_node(&state.store, &node_name).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_unassign",
        node_name = %node_name,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "node_name": node_name,
    }))
}

async fn stop_node(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = node_api::stop_node(&state.store, &node_name).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_stop",
        node_name = %result.node_name,
        rows_updated = result.rows_updated,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "node_name": result.node_name,
        "rows_updated": result.rows_updated,
    }))
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
