mod auth;
mod config_panels;
mod panels;
mod performance_panels;
mod run_exposed_info;
mod run_panels;
mod task_panels;
mod worker_panels;

use crate::api::{
    ApiError, db as db_api, nodes as node_api, runs as run_api, stage as stage_api,
    templates as template_api,
};
use crate::core::{
    AggregationStore, RunReadStore, RunSpecStore, RunTask, RunTaskStore, SamplerQueueTuning,
};
use crate::evaluation::AccumulatorState;
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
use gammalooprs::observables::ObservableSnapshotBundle;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::{
    fs,
    fs::File,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::Instrument;

use self::auth::{AuthConfig, SessionStatus, login, logout, require_admin_session};
use crate::config::{
    DEFAULT_SERVER_CONFIG_PATH, RuntimeConfig, config_base_dir, normalize_config_path,
    read_toml_with_default_fallback,
};

const DEFAULT_SERVER_CONFIG_TOML: &str = include_str!("../config_defaults/server.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub api_host: IpAddr,
    pub api_port: u16,
    pub allowed_origins: Vec<String>,
    pub secure_cookie: bool,
    pub allow_db_admin: bool,
    pub allow_local_node_spawn: bool,
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
        let raw = read_toml_with_default_fallback(
            path,
            DEFAULT_SERVER_CONFIG_PATH,
            DEFAULT_SERVER_CONFIG_TOML,
            "server config",
        )?;
        let mut parsed: Self = toml::from_str(&raw)
            .with_context(|| format!("failed parsing server config {}", path.display()))?;
        let base_dir = config_base_dir(path)?;
        parsed.run_templates_dir =
            normalize_config_path(&base_dir, parsed.run_templates_dir.as_str())
                .display()
                .to_string();
        parsed.task_templates_dir =
            normalize_config_path(&base_dir, parsed.task_templates_dir.as_str())
                .display()
                .to_string();
        Ok(parsed)
    }

    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.api_host, self.api_port)
    }
}

pub async fn serve(
    store: PgStore,
    config: ServerConfig,
    runtime_config_path: PathBuf,
    runtime_config: RuntimeConfig,
) -> anyhow::Result<()> {
    let bind = config.bind_addr();
    let allowed_origins = config
        .allowed_origins
        .iter()
        .map(|origin| {
            axum::http::HeaderValue::from_str(origin.trim())
                .with_context(|| format!("invalid server.allowed_origins entry={origin:?}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if allowed_origins.is_empty() {
        anyhow::bail!("server.allowed_origins must not be empty");
    }
    let state = AppState {
        store,
        auth: AuthConfig::from_server_config(&config.auth),
        allowed_origins,
        secure_cookie: config.secure_cookie,
        allow_db_admin: config.allow_db_admin,
        allow_local_node_spawn: config.allow_local_node_spawn,
        run_templates_dir: PathBuf::from(&config.run_templates_dir),
        task_templates_dir: PathBuf::from(&config.task_templates_dir),
        runtime_cli_args: runtime_cli_args(&runtime_config_path, &runtime_config),
    };

    let app = build_app(state);

    println!("server listening on http://{}", bind);
    println!("api available at http://{}/api", bind);
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
    allowed_origins: Vec<axum::http::HeaderValue>,
    secure_cookie: bool,
    allow_db_admin: bool,
    allow_local_node_spawn: bool,
    run_templates_dir: PathBuf,
    task_templates_dir: PathBuf,
    runtime_cli_args: Vec<String>,
}

fn runtime_cli_args(runtime_config_path: &Path, runtime_config: &RuntimeConfig) -> Vec<String> {
    vec![
        "--runtime-config".to_string(),
        runtime_config_path.display().to_string(),
        "--database-url".to_string(),
        runtime_config.database.url.clone(),
        "--postgres-data-dir".to_string(),
        runtime_config.local_postgres.data_dir.clone(),
        "--postgres-socket-dir".to_string(),
        runtime_config.local_postgres.socket_dir.clone(),
        "--postgres-log-file".to_string(),
        runtime_config.local_postgres.log_file.clone(),
    ]
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
    source: Option<String>,
    run_id: Option<i32>,
    node_name: Option<String>,
    node_uuid: Option<String>,
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

fn log_control_api_error(action: &str, err: &ApiError) {
    match err {
        ApiError::BadRequest(message) => tracing::warn!(
            source = "control",
            control_surface = "dashboard",
            action,
            error = %message,
            "dashboard action rejected"
        ),
        ApiError::Unauthorized(message) => tracing::warn!(
            source = "control",
            control_surface = "dashboard",
            action,
            error = %message,
            "dashboard action unauthorized"
        ),
        ApiError::NotFound(message) => tracing::warn!(
            source = "control",
            control_surface = "dashboard",
            action,
            error = %message,
            "dashboard action target not found"
        ),
        ApiError::Internal(message) => tracing::error!(
            source = "control",
            control_surface = "dashboard",
            action,
            error = %message,
            "dashboard action failed"
        ),
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
struct AutoRunNodesRequest {
    count: usize,
    max_start_failures: Option<u32>,
    #[serde(default)]
    args: JsonValue,
    #[serde(default)]
    name_prefix: Option<String>,
}

#[derive(Deserialize)]
struct NodeLaunchRequestProgressRequest {
    state: String,
    started_count: usize,
    #[serde(default)]
    result: JsonValue,
    #[serde(default)]
    error: Option<String>,
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

#[derive(Deserialize)]
struct UpdateTaskQueueTuningRequest {
    #[serde(default)]
    queue_tuning: Option<SamplerQueueTuning>,
}

#[derive(Deserialize)]
struct TemplateSaveRequest {
    name: String,
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
struct RunReproTomlResponse {
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

#[derive(Deserialize)]
struct HistogramBundleExportRequest {
    payload: JsonValue,
    format: String,
}

#[derive(Serialize)]
struct HistogramBundleExportResponse {
    filename: String,
    mime_type: String,
    contents: String,
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
        .route("/runs/:id/repro-toml", get(get_run_repro_toml))
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
        .route("/logs", get(get_logs))
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
        .route("/histogram-bundle/export", post(export_histogram_bundle));

    let protected_api_routes = Router::new()
        .route("/runs", post(create_run))
        .route("/runs/clone", post(clone_run))
        .route("/runs/:id", delete(delete_run))
        .route("/runs/:id/pause", post(pause_run))
        .route("/runs/:id/tasks", post(add_run_tasks))
        .route(
            "/runs/:id/tasks/:task_id/queue-tuning",
            post(update_run_task_queue_tuning),
        )
        .route("/runs/:id/tasks/:task_id", delete(delete_run_task))
        .route("/runs/:id/auto-assign", post(auto_assign_run))
        .route("/nodes/:id/assign", post(assign_node))
        .route("/nodes/:id/unassign", post(unassign_node))
        .route("/nodes/:id/stop", post(stop_node))
        .route("/nodes/stop-all", post(stop_all_nodes))
        .route("/nodes/auto-run", post(auto_run_nodes))
        .route(
            "/node-launch-requests/claim-external",
            post(claim_external_node_launch_request),
        )
        .route(
            "/node-launch-requests/:id/progress",
            post(update_node_launch_request_progress),
        )
        .route(
            "/node-launch-requests",
            get(get_node_launch_requests).post(create_node_launch_request),
        )
        .route("/templates/runs", post(save_run_template))
        .route("/templates/tasks", post(save_task_template))
        .route("/templates/runs/:name", delete(delete_run_template))
        .route("/templates/tasks/:name", delete(delete_task_template))
        .route("/admin/db/restart", post(restart_db))
        .route("/admin/control/shutdown", post(shutdown_control_process))
        .route("/runs/:id/debug/batches", get(get_run_debug_batches))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_session,
        ));

    Router::new()
        .nest("/api", public_api_routes.merge(protected_api_routes))
        .layer(build_cors_layer(state.allowed_origins.clone()))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state)
}

fn build_cors_layer(allowed_origins: Vec<axum::http::HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::list(allowed_origins))
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
    let workers = state
        .store
        .get_registered_worker_summaries(params.run_id)
        .await?;
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

async fn get_run_repro_toml(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let toml = run_api::export_run_repro_toml(&state.store, id).await?;
    json_response(RunReproTomlResponse { toml })
}

async fn get_run_debug_batches(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<i32>,
    Query(params): Query<HashMap<String, String>>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);
    let payload = state
        .store
        .fetch_pending_claimed_batches_json(run_id, limit)
        .await?;
    json_response(payload)
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
    let exposed_info = crate::core::RunExposedInfoCache::default();
    let tasks = state.store.list_run_tasks(run_id).await?;
    let workers = state.store.get_registered_workers(Some(run_id)).await?;
    json_response(
        build_run_panel_response(&run, &run_spec, &tasks, &workers, &exposed_info)
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

async fn save_run_template(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<TemplateSaveRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template =
        template_api::save_template(&state.run_templates_dir, &payload.name, &payload.toml)?;
    json_response(TemplateFileResponse {
        name: template.name,
        toml: template.toml,
    })
}

async fn save_task_template(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<TemplateSaveRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template =
        template_api::save_template(&state.task_templates_dir, &payload.name, &payload.toml)?;
    json_response(TemplateFileResponse {
        name: template.name,
        toml: template.toml,
    })
}

async fn delete_run_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    template_api::delete_template(&state.run_templates_dir, &name)?;
    json_response(serde_json::json!({ "deleted": true, "name": name }))
}

async fn delete_task_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    template_api::delete_template(&state.task_templates_dir, &name)?;
    json_response(serde_json::json!({ "deleted": true, "name": name }))
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
                domain: &run_spec.domain,
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
    let _run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let run_spec = state
        .store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let sampler_config = if let Some(task) = state.store.load_active_run_task(run_id).await? {
        resolve_active_task_sampler_config(&state, run_id, &task).await?
    } else if let Some(latest_snapshot) = state
        .store
        .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
        .await?
    {
        latest_snapshot.sampler_aggregator.ok_or_else(|| {
            ApiError::BadRequest(format!(
                "run {run_id} has no configured sampler_aggregator yet"
            ))
        })?
    } else {
        return Err(ApiError::BadRequest(format!(
            "run {run_id} has no configured sampler_aggregator yet"
        )));
    };
    let response: PanelResponse = sampler_config
        .build_response(
            format!("run:{run_id}:config:sampler_aggregator"),
            &SamplerAggregatorPanelContext {
                domain: &run_spec.domain,
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
) -> Result<crate::core::SamplerAggregatorConfig, ApiError> {
    if let Some(config) = task.task.sampler_config() {
        return Ok(config);
    }
    if let Some(config) = task.task.sample_sampler_config() {
        return Ok(config);
    }

    if let Some(source_snapshot) = stage_api::resolve_task_source_snapshot(
        &state.store,
        run_id,
        task,
        task.task.sample_sampler_source(),
    )
    .await?
    {
        if let Some(config) = source_snapshot.sampler_aggregator {
            return Ok(config);
        }
    }

    if let Some(base_snapshot) = state
        .store
        .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
        .await?
    {
        if let Some(config) = base_snapshot.sampler_aggregator {
            return Ok(config);
        }
    }

    Err(ApiError::BadRequest(format!(
        "task {} has no effective sampler_aggregator configuration",
        task.id
    )))
}

async fn get_run_task_output(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
    AxumJson(request): AxumJson<TaskPanelRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(request.limit);
    let cursor =
        parse_task_panel_cursor(request.request.cursor.as_deref()).map_err(ApiError::BadRequest)?;
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let effective_accumulator_config =
        if matches!(task.task, crate::core::RunTaskSpec::Sample { .. }) {
            stage_api::try_resolve_effective_sample_accumulator_config(&state.store, run_id, &task)
                .await?
        } else {
            None
        };
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
    let current_accumulator = if matches!(task.state, crate::core::RunTaskState::Active) {
        state
            .store
            .load_current_accumulator(run_id)
            .await?
            .map(|current_accumulator| {
                AccumulatorState::from_json(&current_accumulator)
                    .map_err(|err| ApiError::Internal(err.to_string()))
            })
            .transpose()?
    } else {
        None
    };
    let panel_source = TaskPanelSource::new(&task.task, effective_accumulator_config)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let delta_history_snapshots = if panel_source.needs_history() && cursor.snapshot_id.is_some() {
        state
            .store
            .get_task_output_snapshots(run_id, task.id, cursor.snapshot_id, limit)
            .await?
    } else {
        Vec::new()
    };
    let full_history_snapshots = if panel_source.needs_history() && cursor.snapshot_id.is_none() {
        state
            .store
            .get_task_output_snapshots(run_id, task.id, None, i64::MAX)
            .await?
    } else {
        Vec::new()
    };
    let (completed_samples_per_second, smoothed_eta_seconds) =
        if matches!(task.task, crate::core::RunTaskSpec::Sample { .. })
            && matches!(task.state, crate::core::RunTaskState::Active)
        {
            let metrics = state
                .store
                .get_sampler_performance_history(run_id, 1, None)
                .await?
                .first()
                .and_then(|entry| {
                    serde_json::from_value::<crate::core::SamplerRuntimeMetrics>(
                        entry.runtime_metrics.clone(),
                    )
                    .ok()
                });
            let completed_samples_per_second = metrics
                .as_ref()
                .map(|metrics| {
                    if metrics.eta_completed_samples_per_second.is_finite()
                        && metrics.eta_completed_samples_per_second > 0.0
                    {
                        metrics.eta_completed_samples_per_second
                    } else {
                        metrics.completed_samples_per_second
                    }
                })
                .filter(|value| value.is_finite() && *value > 0.0);
            let smoothed_eta_seconds = metrics
                .as_ref()
                .and_then(|metrics| metrics.eta_seconds_smoothed)
                .filter(|value| value.is_finite() && *value >= 0.0);
            (completed_samples_per_second, smoothed_eta_seconds)
        } else {
            (None, None)
        };

    let payload = panel_source
        .build_response(
            format!("run:{run_id}:task:{}", task.id),
            cursor,
            &task,
            &request.request.panel_state,
            run.target.as_ref(),
            completed_samples_per_second,
            smoothed_eta_seconds,
            current_accumulator.as_ref(),
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
        .get_runtime_logs(
            limit,
            params.source.as_deref(),
            Some(id),
            params.node_name.as_deref(),
            params.node_uuid.as_deref(),
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

async fn get_logs(
    State(state): State<AppState>,
    Query(params): Query<LogQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let logs = state
        .store
        .get_runtime_logs(
            limit,
            params.source.as_deref(),
            params.run_id,
            params.node_name.as_deref(),
            params.node_uuid.as_deref(),
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
    let config = run_api::parse_run_add_config_toml(payload.toml.trim()).map_err(|err| {
        log_control_api_error("run_create", &err);
        err
    })?;
    let run = run_api::create_run(&state.store, config)
        .await
        .map_err(|err| {
            log_control_api_error("run_create", &err);
            err
        })?;
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
    .await
    .map_err(|err| {
        log_control_api_error("run_clone", &err);
        err
    })?;
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
    let tasks = run_api::parse_task_queue_toml(payload.toml.trim()).map_err(|err| {
        log_control_api_error("run_add_tasks", &err);
        err
    })?;
    let result = run_api::append_tasks(&state.store, run_id, tasks)
        .await
        .map_err(|err| {
            log_control_api_error("run_add_tasks", &err);
            err
        })?;
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
    let result = run_api::pause_run(&state.store, run_id)
        .await
        .map_err(|err| {
            log_control_api_error("run_pause", &err);
            err
        })?;
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
    let result = run_api::remove_run(&state.store, run_id)
        .await
        .map_err(|err| {
            log_control_api_error("run_remove", &err);
            err
        })?;
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
    let result = run_api::remove_pending_task(&state.store, run_id, task_id)
        .await
        .map_err(|err| {
            log_control_api_error("run_task_remove", &err);
            err
        })?;
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

async fn update_run_task_queue_tuning(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
    AxumJson(payload): AxumJson<UpdateTaskQueueTuningRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result =
        run_api::update_task_queue_tuning(&state.store, run_id, task_id, payload.queue_tuning)
            .await
            .map_err(|err| {
                log_control_api_error("run_task_update_queue_tuning", &err);
                err
            })?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "run_task_update_queue_tuning",
        run_id = result.run_id,
        task_id = result.task.id,
        "dashboard action completed"
    );
    json_response(result.task)
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
    .await
    .map_err(|err| {
        log_control_api_error("node_assign", &err);
        err
    })?;
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
    let result = node_api::auto_assign_run(&state.store, run_id, payload.max_evaluators)
        .await
        .map_err(|err| {
            log_control_api_error("run_auto_assign", &err);
            err
        })?;

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
    node_api::unassign_node(&state.store, &node_name)
        .await
        .map_err(|err| {
            log_control_api_error("node_unassign", &err);
            err
        })?;
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
    let result = node_api::stop_node(&state.store, &node_name)
        .await
        .map_err(|err| {
            log_control_api_error("node_stop", &err);
            err
        })?;
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

async fn stop_all_nodes(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = node_api::stop_all_nodes(&state.store)
        .await
        .map_err(|err| {
            log_control_api_error("node_stop_all", &err);
            err
        })?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_stop_all",
        rows_updated = result.rows_updated,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "rows_updated": result.rows_updated,
    }))
}

async fn auto_run_nodes(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<AutoRunNodesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    create_and_maybe_resolve_node_launch_request(state, payload).await
}

async fn create_node_launch_request(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<AutoRunNodesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    create_and_maybe_resolve_node_launch_request(state, payload).await
}

async fn get_node_launch_requests(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    node_api::reconcile_running_node_launch_requests(&state.store)
        .await
        .map_err(|err| {
            log_control_api_error("node_launch_requests_reconcile", &err);
            err
        })?;
    let requests = node_api::list_node_launch_requests(&state.store)
        .await
        .map_err(|err| {
            log_control_api_error("node_launch_requests_list", &err);
            err
        })?;
    json_response(serde_json::json!({ "items": requests }))
}

async fn claim_external_node_launch_request(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = node_api::claim_external_node_launch_request(&state.store)
        .await
        .map_err(|err| {
            log_control_api_error("node_launch_request_claim_external", &err);
            err
        })?;
    json_response(serde_json::json!({ "request": request }))
}

async fn update_node_launch_request_progress(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    AxumJson(payload): AxumJson<NodeLaunchRequestProgressRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = if payload.result.is_null() {
        serde_json::json!({})
    } else {
        payload.result
    };
    let request = match payload.state.as_str() {
        "starting" => {
            node_api::mark_node_launch_request_starting(
                &state.store,
                id,
                payload.started_count,
                &result,
            )
            .await
        }
        "running" => {
            node_api::mark_node_launch_request_running(
                &state.store,
                id,
                payload.started_count,
                &result,
            )
            .await
        }
        "failed" => {
            let error = payload
                .error
                .as_deref()
                .unwrap_or("external launcher reported failure");
            node_api::mark_node_launch_request_failed(
                &state.store,
                id,
                payload.started_count,
                &result,
                error,
            )
            .await
        }
        "canceled" => {
            node_api::mark_node_launch_request_canceled(
                &state.store,
                id,
                payload.started_count,
                &result,
            )
            .await
        }
        other => Err(ApiError::BadRequest(format!(
            "unsupported node launch request state '{other}'"
        ))),
    }
    .map_err(|err| {
        log_control_api_error("node_launch_request_progress", &err);
        err
    })?;
    json_response(serde_json::json!({ "request": request }))
}

async fn create_and_maybe_resolve_node_launch_request(
    state: AppState,
    payload: AutoRunNodesRequest,
) -> Result<Json<serde_json::Value>, ApiError> {
    let max_start_failures = payload.max_start_failures.unwrap_or(3);
    let backend = if state.allow_local_node_spawn {
        "local"
    } else {
        "external"
    };
    let args = match payload.args {
        JsonValue::Object(mut map) => {
            map.insert(
                "max_start_failures".to_string(),
                JsonValue::from(max_start_failures),
            );
            JsonValue::Object(map)
        }
        JsonValue::Null => serde_json::json!({ "max_start_failures": max_start_failures }),
        other => serde_json::json!({
            "max_start_failures": max_start_failures,
            "payload": other,
        }),
    };
    let launch = node_api::create_node_launch_request(
        &state.store,
        payload.count,
        backend,
        payload.name_prefix.as_deref(),
        &args,
        state.allow_local_node_spawn,
    )
    .await
    .map_err(|err| {
        log_control_api_error("node_launch_request_create", &err);
        err
    })?;

    if !launch.should_resolve_locally {
        tracing::info!(
            source = "control",
            control_surface = "dashboard",
            action = "node_launch_request_create",
            request_id = launch.request.id,
            requested = launch.request.requested_count,
            backend = %launch.request.backend,
            "dashboard action completed"
        );
        return json_response(serde_json::json!({
            "request": launch.request,
            "requested": payload.count,
            "started": 0,
            "node_names": [],
        }));
    }

    let plan = node_api::plan_auto_run_nodes(&state.store, payload.count)
        .await
        .map_err(|err| {
            log_control_api_error("node_auto_run", &err);
            err
        })?;

    let binary = std::env::current_exe().map_err(|err| {
        ApiError::Internal(format!("failed to resolve current executable: {err}"))
    })?;

    let mut started_node_names = Vec::new();
    for node_name in &plan.node_names {
        if let Err(err) = spawn_node_process(
            &binary,
            &state.runtime_cli_args,
            node_name,
            max_start_failures,
        ) {
            let workers = started_node_names
                .iter()
                .map(|node_name| serde_json::json!({ "node_name": node_name }))
                .collect::<Vec<_>>();
            let result = serde_json::json!({ "workers": workers });
            let _ = node_api::mark_node_launch_request_failed(
                &state.store,
                launch.request.id,
                started_node_names.len(),
                &result,
                &err.to_string(),
            )
            .await;
            log_control_api_error("node_auto_run", &err);
            return Err(err);
        }
        started_node_names.push(node_name.clone());
    }

    let workers = started_node_names
        .iter()
        .map(|node_name| serde_json::json!({ "node_name": node_name }))
        .collect::<Vec<_>>();
    let result = serde_json::json!({ "workers": workers });
    let request = node_api::mark_node_launch_request_starting(
        &state.store,
        launch.request.id,
        started_node_names.len(),
        &result,
    )
    .await
    .map_err(|err| {
        log_control_api_error("node_launch_request_starting", &err);
        err
    })?;

    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_auto_run",
        request_id = launch.request.id,
        requested = plan.requested_count,
        started = started_node_names.len(),
        node_names = ?started_node_names,
        max_start_failures,
        "dashboard action completed"
    );

    json_response(serde_json::json!({
        "request": request,
        "requested": plan.requested_count,
        "started": started_node_names.len(),
        "node_names": started_node_names,
    }))
}

async fn restart_db(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.allow_db_admin {
        return Err(ApiError::BadRequest(
            "database admin endpoints are disabled by server config".to_string(),
        ));
    }

    let binary = std::env::current_exe().map_err(|err| {
        ApiError::Internal(format!("failed to resolve current executable: {err}"))
    })?;
    let result =
        db_api::restart_local_database(&binary, &state.runtime_cli_args).map_err(|err| {
            log_control_api_error("db_restart", &err);
            err
        })?;

    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "db_restart",
        deleted = result.deleted,
        started = result.started,
        "dashboard action completed"
    );

    json_response(serde_json::json!({
        "deleted": result.deleted,
        "started": result.started,
    }))
}

async fn shutdown_control_process(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.allow_db_admin {
        return Err(ApiError::BadRequest(
            "control admin endpoints are disabled by server config".to_string(),
        ));
    }

    tracing::warn!(
        source = "control",
        control_surface = "dashboard",
        action = "control_shutdown",
        "dashboard action requested control process shutdown"
    );
    let store = state.store.clone();
    tokio::spawn(async move {
        let result = node_api::stop_all_nodes_gracefully(
            &store,
            node_api::GracefulNodeShutdownParams {
                sampler_drain_timeout: Duration::from_secs(60),
                node_stop_timeout: Duration::from_secs(15),
                poll_interval: Duration::from_millis(250),
            },
        )
        .await;
        match result {
            Ok(result) => tracing::info!(
                source = "control",
                control_surface = "dashboard",
                action = "control_shutdown",
                assignments_cleared = result.assignments_cleared,
                rows_updated = result.rows_updated,
                active_samplers_remaining = result.active_samplers_remaining,
                live_nodes_remaining = result.live_nodes_remaining,
                sampler_drain_timed_out = result.sampler_drain_timed_out,
                node_stop_timed_out = result.node_stop_timed_out,
                "control shutdown drain completed"
            ),
            Err(err) => tracing::error!(
                source = "control",
                control_surface = "dashboard",
                action = "control_shutdown",
                error = %err,
                "control shutdown drain failed"
            ),
        }
        std::process::exit(0);
    });

    json_response(serde_json::json!({ "shutdown_requested": true }))
}

fn spawn_node_process(
    binary: &Path,
    runtime_cli_args: &[String],
    node_name: &str,
    max_start_failures: u32,
) -> Result<(), ApiError> {
    use std::process::Stdio;
    use tokio::process::Command;

    let (stdout_log_path, stderr_log_path) = node_process_log_paths(node_name)?;
    let stdout_log = File::create(&stdout_log_path).map_err(|err| {
        ApiError::Internal(format!(
            "failed to open stdout log for node {node_name} at {}: {err}",
            stdout_log_path.display()
        ))
    })?;
    let stderr_log = File::create(&stderr_log_path).map_err(|err| {
        ApiError::Internal(format!(
            "failed to open stderr log for node {node_name} at {}: {err}",
            stderr_log_path.display()
        ))
    })?;

    let mut command = Command::new(binary);
    command
        .args(runtime_cli_args)
        .args(node_api::node_run_cli_args(node_name, max_start_failures))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log));

    let mut child = command
        .spawn()
        .map_err(|err| ApiError::Internal(format!("failed to spawn node {node_name}: {err}")))?;
    let name = node_name.to_string();
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) if !status.success() => {
                tracing::warn!(
                    node_name = %name,
                    exit_status = %status,
                    stdout_log = %stdout_log_path.display(),
                    stderr_log = %stderr_log_path.display(),
                    "spawned node process exited unsuccessfully"
                );
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    node_name = %name,
                    error = %err,
                    stdout_log = %stdout_log_path.display(),
                    stderr_log = %stderr_log_path.display(),
                    "spawned node process wait failed"
                );
            }
        }
    });
    Ok(())
}

fn node_process_log_paths(node_name: &str) -> Result<(PathBuf, PathBuf), ApiError> {
    let dir = PathBuf::from("logs").join("nodes");
    fs::create_dir_all(&dir).map_err(|err| {
        ApiError::Internal(format!(
            "failed to create node log directory {}: {err}",
            dir.display()
        ))
    })?;
    Ok((
        dir.join(format!("{node_name}.stdout.log")),
        dir.join(format!("{node_name}.stderr.log")),
    ))
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
    json_response(build_evaluator_performance_response(
        Some(scope_id),
        rows,
        params.node_name.is_none(),
    ))
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
        false,
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

async fn export_histogram_bundle(
    AxumJson(request): AxumJson<HistogramBundleExportRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let format = request.format.trim().to_ascii_lowercase();
    let wrapper = if request.payload.get("histograms").is_some() {
        request.payload
    } else {
        serde_json::json!({
            "histograms": request.payload,
        })
    };
    let primary_histogram_name = wrapper
        .get("primary_histogram_name")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let bundle: ObservableSnapshotBundle = serde_json::from_value(wrapper).map_err(|err| {
        ApiError::BadRequest(format!(
            "invalid histogram bundle payload for export: {err}"
        ))
    })?;

    let response = match format.as_str() {
        "json" => {
            let payload = serde_json::json!({
                "primary_histogram_name": primary_histogram_name,
                "histograms": bundle.histograms,
            });
            let contents = serde_json::to_string_pretty(&payload).map_err(|err| {
                ApiError::Internal(format!("failed to serialize histogram bundle json: {err}"))
            })?;
            HistogramBundleExportResponse {
                filename: "histogram_bundle.json".to_string(),
                mime_type: "application/json;charset=utf-8".to_string(),
                contents: format!("{contents}\n"),
            }
        }
        "hwu" => {
            let file = tempfile::NamedTempFile::new().map_err(|err| {
                ApiError::Internal(format!("failed to create temporary hwu file: {err}"))
            })?;
            bundle
                .write_hwu_file(file.path())
                .map_err(|err| ApiError::Internal(format!("failed to export HwU bundle: {err}")))?;
            let contents = fs::read_to_string(file.path()).map_err(|err| {
                ApiError::Internal(format!("failed to read exported HwU bundle: {err}"))
            })?;
            HistogramBundleExportResponse {
                filename: "histogram_bundle.HwU".to_string(),
                mime_type: "text/plain;charset=utf-8".to_string(),
                contents,
            }
        }
        _ => {
            return Err(ApiError::BadRequest(
                "unsupported histogram export format (expected 'json' or 'hwu')".to_string(),
            ));
        }
    };

    json_response(response)
}
