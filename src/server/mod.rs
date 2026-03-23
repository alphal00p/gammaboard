mod auth;
mod config_panels;
mod panels;
mod performance_panels;
mod run_panels;
mod task_panels;
mod worker_panels;

use crate::core::{
    AggregationStore, ControlPlaneStore, IntegrationParams, RunReadStore, RunSpecStore,
    RunStageSnapshot, RunTask, RunTaskInputSpec, RunTaskSpec, RunTaskStore, StageSnapshotRef,
    StoreError, WorkerRole,
};
use crate::evaluation::ObservableState;
use crate::preprocess::{RunAddConfig, preflight_task_suffix, preprocess_run_add};
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
    let shutdown = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to install server Ctrl-C handler: {err}");
            return;
        }
        tracing::info!("server shutdown requested by Ctrl-C");
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
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

fn decode_integration_params(
    run_id: i32,
    value: serde_json::Value,
) -> Result<IntegrationParams, ApiError> {
    serde_json::from_value(value).map_err(|err| {
        ApiError::Internal(format!(
            "invalid integration_params for run {run_id}: {err}"
        ))
    })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
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
    from_snapshot_id: i64,
    new_name: String,
}

#[derive(Deserialize)]
struct AddTasksRequest {
    toml: String,
}

#[derive(Debug, Deserialize)]
struct TaskQueueFile {
    task_queue: Vec<RunTaskInputSpec>,
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
    latest_stage_snapshot_id: Option<i64>,
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
        .route("/runs/:id/pause", post(pause_run))
        .route("/runs/:id/tasks", post(add_run_tasks))
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
    let response = tasks
        .into_iter()
        .map(|task| RunTaskResponse {
            latest_stage_snapshot_id: latest_snapshot_ids.get(&task.id).copied(),
            task,
        })
        .collect::<Vec<_>>();
    json_response(response)
}

async fn list_run_templates(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: list_template_files(&state.run_templates_dir)?,
    })
}

async fn get_run_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(read_template_file(&state.run_templates_dir, &name)?)
}

async fn list_task_templates(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: list_template_files(&state.task_templates_dir)?,
    })
}

async fn get_task_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(read_template_file(&state.task_templates_dir, &name)?)
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
    let sampler_config = if let Some(task) = state.store.load_active_run_task(run_id).await? {
        task.task.sampler_config().ok_or_else(|| {
            ApiError::BadRequest(format!(
                "task {} does not define a sampler_aggregator config",
                task.id
            ))
        })?
    } else {
        let run = state
            .store
            .get_run_progress(run_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
        let integration_params = decode_integration_params(
            run_id,
            run.integration_params.ok_or_else(|| {
                ApiError::Internal(format!("run {run_id} is missing integration_params"))
            })?,
        )?;
        integration_params.sampler_aggregator
    };
    let response: PanelResponse = sampler_config
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

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/default.toml";

fn read_default_run_add_toml() -> Result<toml::Value, ApiError> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let raw = fs::read_to_string(&path).map_err(|err| {
        ApiError::Internal(format!(
            "failed reading default run config {}: {err}",
            path.display()
        ))
    })?;
    toml::from_str(&raw).map_err(|err| {
        ApiError::Internal(format!(
            "failed parsing default run config {}: {err}",
            path.display()
        ))
    })
}

fn list_template_files(dir: &Path) -> Result<Vec<String>, ApiError> {
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(items);
    };
    for entry in entries {
        let entry = entry.map_err(|err| {
            ApiError::Internal(format!(
                "failed reading template dir {}: {err}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        items.push(name.to_string());
    }
    items.sort();
    Ok(items)
}

fn read_template_file(dir: &Path, name: &str) -> Result<TemplateFileResponse, ApiError> {
    let path = resolve_template_path(dir, name)?;
    let toml = fs::read_to_string(&path)
        .map_err(|err| ApiError::Internal(format!("failed reading {}: {err}", path.display())))?;
    Ok(TemplateFileResponse {
        name: name.to_string(),
        toml,
    })
}

fn resolve_template_path(dir: &Path, name: &str) -> Result<PathBuf, ApiError> {
    if name.is_empty()
        || !name.ends_with(".toml")
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name)
    {
        return Err(ApiError::BadRequest("invalid template name".to_string()));
    }
    let path = dir.join(name);
    if !path.is_file() {
        return Err(ApiError::NotFound(format!("template {name} not found")));
    }
    Ok(path)
}

fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                if let Some(base_value) = base_table.get_mut(&key) {
                    merge_toml(base_value, value);
                } else {
                    base_table.insert(key, value);
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value;
        }
    }
}

fn parse_run_add_config(raw: &str) -> Result<RunAddConfig, ApiError> {
    let mut merged = read_default_run_add_toml()?;
    let overlay = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("failed parsing run TOML: {err}")))?;
    merge_toml(&mut merged, overlay);
    if merged
        .as_table()
        .and_then(|table| table.get("point_spec"))
        .is_some()
    {
        return Err(ApiError::BadRequest(
            "top-level [point_spec] is no longer supported; define dimensions in [evaluator]"
                .to_string(),
        ));
    }
    let parsed: RunAddConfig = merged
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid run-add payload: {err}")))?;
    let name = parsed.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "invalid run name (`name`): expected non-empty string".to_string(),
        ));
    }
    if let Some(task_queue) = parsed.task_queue.as_ref() {
        if task_queue.is_empty() {
            return Err(ApiError::BadRequest(
                "invalid task_queue: expected at least one task when set".to_string(),
            ));
        }
        for task in task_queue {
            task.validate()
                .map_err(|err| ApiError::BadRequest(format!("invalid task_queue entry: {err}")))?;
        }
    }
    Ok(RunAddConfig { name, ..parsed })
}

fn parse_task_queue_payload(raw: &str) -> Result<TaskQueueFile, ApiError> {
    let parsed: TaskQueueFile = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))?;
    if parsed.task_queue.is_empty() {
        return Err(ApiError::BadRequest(
            "invalid task_queue: expected at least one task".to_string(),
        ));
    }
    for task in &parsed.task_queue {
        task.validate()
            .map_err(|err| ApiError::BadRequest(format!("invalid task_queue entry: {err}")))?;
    }
    Ok(parsed)
}

async fn resolve_task_queue_payload_for_run(
    store: &PgStore,
    run_id: i32,
    raw: &str,
) -> Result<Vec<RunTaskSpec>, ApiError> {
    let parsed = parse_task_queue_payload(raw)?;
    let _run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let base_snapshot = load_append_base_snapshot(store, run_id).await?;
    crate::core::resolve_task_queue(
        &base_snapshot.sampler_aggregator,
        &base_snapshot.parametrization.config,
        &parsed.task_queue,
    )
    .map_err(ApiError::BadRequest)
}

async fn load_append_base_snapshot(
    store: &PgStore,
    run_id: i32,
) -> Result<RunStageSnapshot, ApiError> {
    store
        .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} has no base stage snapshot")))
}

async fn preflight_task_batch(
    store: &PgStore,
    base_snapshot: &RunStageSnapshot,
    integration_params: &IntegrationParams,
    tasks: &[RunTaskSpec],
    point_spec: &crate::evaluation::PointSpec,
) -> Result<(), ApiError> {
    let mut referenced_snapshots = std::collections::BTreeMap::new();
    for task in tasks {
        if let Some(start_from) = task.start_from() {
            let snapshot = store
                .load_stage_snapshot(start_from.snapshot_id)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "task start_from references snapshot {} but no stage snapshot exists",
                        start_from.snapshot_id
                    ))
                })?;
            referenced_snapshots.insert(start_from.snapshot_id, snapshot);
        }
    }
    let mut evaluator = integration_params
        .evaluator
        .build()
        .map_err(|err| ApiError::BadRequest(format!("failed to build evaluator: {err}")))?;
    preflight_task_suffix(
        base_snapshot,
        &referenced_snapshots,
        tasks,
        &mut *evaluator,
        point_spec,
    )
    .map_err(|err| ApiError::BadRequest(format!("failed to preflight task batch: {err}")))
}

fn clone_task_suffix(
    source_tasks: &[RunTask],
    from_snapshot: &RunStageSnapshot,
) -> Result<Vec<RunTaskSpec>, ApiError> {
    let source_index = match from_snapshot.task_id {
        Some(task_id) => Some(
            source_tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| ApiError::NotFound(format!("run task {task_id} not found")))?,
        ),
        None => None,
    };
    let mut cloned_tasks = source_tasks
        .iter()
        .skip(source_index.map_or(0, |index| index + 1))
        .map(|task| task.task.clone())
        .collect::<Vec<_>>();
    if let Some(first_executable) = cloned_tasks
        .iter_mut()
        .find(|task| !matches!(task, RunTaskSpec::Pause))
    {
        set_task_start_from(
            first_executable,
            StageSnapshotRef {
                snapshot_id: from_snapshot.id.ok_or_else(|| {
                    ApiError::Internal("source stage snapshot is missing id".to_string())
                })?,
            },
        );
    }
    Ok(cloned_tasks)
}

fn set_task_start_from(task: &mut RunTaskSpec, start_from: StageSnapshotRef) {
    match task {
        RunTaskSpec::Sample {
            start_from: task_start_from,
            ..
        }
        | RunTaskSpec::Image {
            start_from: task_start_from,
            ..
        }
        | RunTaskSpec::PlotLine {
            start_from: task_start_from,
            ..
        } => {
            *task_start_from = Some(start_from);
        }
        RunTaskSpec::Pause => {}
    }
}

async fn create_run(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<CreateRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_config = parse_run_add_config(payload.toml.trim())?;
    let processed = preprocess_run_add(run_config)
        .map_err(|err| ApiError::BadRequest(format!("failed to preprocess run config: {err}")))?;
    let point_spec = processed.point_spec.as_ref().ok_or_else(|| {
        ApiError::Internal("preprocessing did not resolve point_spec".to_string())
    })?;
    let integration_params = serde_json::to_value(
        processed
            .resolved_integration_params
            .as_ref()
            .ok_or_else(|| {
                ApiError::Internal("preprocessing did not resolve integration_params".to_string())
            })?,
    )
    .map_err(|err| ApiError::Internal(format!("failed to serialize integration_params: {err}")))?;
    let initial_tasks = processed.resolved_task_queue.clone().unwrap_or_default();
    let initial_stage_snapshot = processed.initial_stage_snapshot.as_ref().ok_or_else(|| {
        ApiError::Internal("preprocessing did not build initial stage snapshot".to_string())
    })?;
    preflight_task_batch(
        &state.store,
        initial_stage_snapshot,
        processed
            .resolved_integration_params
            .as_ref()
            .ok_or_else(|| {
                ApiError::Internal("preprocessing did not resolve integration_params".to_string())
            })?,
        &initial_tasks,
        point_spec,
    )
    .await?;
    let run_id = state
        .store
        .create_run(
            &processed.name,
            &integration_params,
            processed.target.as_ref(),
            point_spec,
            processed.evaluator_init_metadata.as_ref(),
            processed.sampler_aggregator_init_metadata.as_ref(),
            initial_stage_snapshot,
            &initial_tasks,
        )
        .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_create",
        run_id,
        run_name = %processed.name,
        tasks_created = initial_tasks.len(),
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run_id,
        "run_name": processed.name,
    }))
}

async fn clone_run(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<CloneRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let new_name = payload.new_name.trim().to_string();
    if new_name.is_empty() {
        return Err(ApiError::BadRequest(
            "invalid run name (`new_name`): expected non-empty string".to_string(),
        ));
    }
    let from_snapshot_id = payload.from_snapshot_id;
    let source_run = state
        .store
        .get_run_progress(payload.source_run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {} not found", payload.source_run_id)))?;
    let point_spec = source_run.point_spec.clone().ok_or_else(|| {
        ApiError::Internal(format!(
            "source run {} is missing point_spec",
            payload.source_run_id
        ))
    })?;
    let integration_params = source_run.integration_params.clone().ok_or_else(|| {
        ApiError::Internal(format!(
            "source run {} is missing integration_params",
            payload.source_run_id
        ))
    })?;
    let snapshot = state.store.load_stage_snapshot(from_snapshot_id).await?;
    let snapshot = snapshot.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "cannot clone from snapshot {}: no stage snapshot exists",
            from_snapshot_id
        ))
    })?;
    if snapshot.run_id != payload.source_run_id {
        return Err(ApiError::BadRequest(format!(
            "snapshot {} belongs to run {}, not source run {}",
            from_snapshot_id, snapshot.run_id, payload.source_run_id
        )));
    }
    let source_tasks = state.store.list_run_tasks(payload.source_run_id).await?;
    let cloned_tasks = clone_task_suffix(&source_tasks, &snapshot)?;
    let run_id = state
        .store
        .create_run(
            &new_name,
            &integration_params,
            source_run.target.as_ref(),
            &point_spec,
            source_run.evaluator_init_metadata.as_ref(),
            source_run.sampler_aggregator_init_metadata.as_ref(),
            &RunStageSnapshot {
                id: None,
                run_id: 0,
                task_id: None,
                sequence_nr: None,
                queue_empty: snapshot.queue_empty,
                sampler_snapshot: snapshot.sampler_snapshot.clone(),
                observable_state: snapshot.observable_state.clone(),
                sampler_aggregator: snapshot.sampler_aggregator.clone(),
                parametrization: snapshot.parametrization.clone(),
            },
            &cloned_tasks,
        )
        .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_clone",
        run_id,
        new_name = %new_name,
        source_run_id = payload.source_run_id,
        from_snapshot_id,
        cloned_tasks = cloned_tasks.len(),
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run_id,
        "run_name": new_name,
    }))
}

async fn add_run_tasks(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
    AxumJson(payload): AxumJson<AddTasksRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tasks =
        resolve_task_queue_payload_for_run(&state.store, run_id, payload.toml.trim()).await?;
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let point_spec = run
        .point_spec
        .as_ref()
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} is missing point_spec")))?;
    let integration_params = decode_integration_params(
        run_id,
        run.integration_params.ok_or_else(|| {
            ApiError::Internal(format!("run {run_id} is missing integration_params"))
        })?,
    )?;
    let base_snapshot = load_append_base_snapshot(&state.store, run_id).await?;
    preflight_task_batch(
        &state.store,
        &base_snapshot,
        &integration_params,
        &tasks,
        point_spec,
    )
    .await?;
    let inserted = state.store.append_run_tasks(run_id, &tasks).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_add_tasks",
        run_id,
        tasks_added = inserted.len(),
        "dashboard action completed"
    );
    json_response(inserted)
}

async fn pause_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let cleared = state
        .store
        .clear_desired_assignments_for_run(run_id)
        .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_pause",
        run_id,
        run_name = %run.run_name,
        assignments_cleared = cleared,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run_id,
        "assignments_cleared": cleared,
    }))
}

async fn assign_node(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
    AxumJson(payload): AxumJson<AssignNodeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let role: WorkerRole = payload
        .role
        .parse()
        .map_err(|err: String| ApiError::BadRequest(err))?;
    let run = state
        .store
        .get_run_progress(payload.run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {} not found", payload.run_id)))?;
    state
        .store
        .upsert_desired_assignment(&node_name, role, payload.run_id)
        .await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_assign",
        node_name = %node_name,
        run_id = payload.run_id,
        run_name = %run.run_name,
        role = %role,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "node_name": node_name,
        "run_id": payload.run_id,
        "role": role.as_str(),
    }))
}

async fn auto_assign_run(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
    AxumJson(payload): AxumJson<AutoAssignRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let nodes = state.store.list_nodes(None).await?;
    let free_nodes = nodes
        .iter()
        .filter(|node| node.desired_assignment.is_none())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();
    let sampler_already_assigned = nodes.iter().any(|node| {
        node.desired_assignment.as_ref().is_some_and(|assignment| {
            assignment.run_id == run_id && assignment.role == WorkerRole::SamplerAggregator
        })
    });

    let evaluator_limit = payload.max_evaluators.unwrap_or(usize::MAX);
    let mut assigned_sampler = None;
    let mut assigned_evaluators = Vec::new();
    let mut free_iter = free_nodes.into_iter();

    if !sampler_already_assigned {
        if let Some(node_name) = free_iter.next() {
            state
                .store
                .upsert_desired_assignment(&node_name, WorkerRole::SamplerAggregator, run_id)
                .await?;
            assigned_sampler = Some(node_name);
        }
    }

    for node_name in free_iter.take(evaluator_limit) {
        state
            .store
            .upsert_desired_assignment(&node_name, WorkerRole::Evaluator, run_id)
            .await?;
        assigned_evaluators.push(node_name);
    }

    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_auto_assign",
        run_id,
        run_name = %run.run_name,
        sampler_already_assigned,
        assigned_sampler = assigned_sampler.as_deref().unwrap_or("none"),
        assigned_evaluators = assigned_evaluators.len(),
        requested_evaluator_limit = payload.max_evaluators,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "run_id": run_id,
        "sampler_already_assigned": sampler_already_assigned,
        "assigned_sampler": assigned_sampler,
        "assigned_evaluators": assigned_evaluators,
    }))
}

async fn unassign_node(
    State(state): State<AppState>,
    AxumPath(node_name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store.clear_desired_assignment(&node_name).await?;
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
    let rows = state.store.request_node_shutdown(&node_name).await?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_stop",
        node_name = %node_name,
        rows_updated = rows,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "node_name": node_name,
        "rows_updated": rows,
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
