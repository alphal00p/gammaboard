mod auth;
mod config_panels;
mod panels;
mod performance_panels;
mod run_panels;
mod settings;
mod task_panels;
mod worker_panels;

use crate::api::{
    ApiError, db as db_api, nodes as node_api, runs as run_api, stage as stage_api,
    templates as template_api, toml_template,
};
use crate::core::{
    AggregationStore, ControlPlaneStore, RunReadStore, RunSpecStore, RunTask, RunTaskStore,
    SamplerQueueTuning,
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
    extract::{DefaultBodyLimit, Json as AxumJson, Path as AxumPath, Query, State},
    http::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
};
#[cfg(feature = "gammaloop")]
use gammalooprs::observables::ObservableSnapshotBundle;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(feature = "gammaloop")]
use std::fs;
use std::{
    fs::File,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::Instrument;

use self::auth::{AuthConfig, SessionStatus, login, logout, require_admin_session};
use crate::config::{
    DEFAULT_SERVER_CONFIG_PATH, config_base_dir, normalize_config_path,
    read_toml_with_default_fallback,
};
use crate::resources::primary_resource_root;
use crate::runtime_context::RuntimeContext;

const DEFAULT_SERVER_CONFIG_TOML: &str = include_str!("../config_defaults/server.toml");
const REPOSITORY_URL: &str = "https://github.com/alphal00p/gammaboard";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(skip)]
    pub server_config_path: PathBuf,
    #[serde(default = "default_server_name")]
    pub name: String,
    #[serde(default = "default_api_host")]
    pub api_host: IpAddr,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub secure_cookie: bool,
    #[serde(default = "default_allow_local_node_spawn")]
    pub allow_local_node_spawn: bool,
    #[serde(default = "default_run_templates_dir")]
    pub run_templates_dir: String,
    #[serde(default = "default_task_templates_dir")]
    pub task_templates_dir: String,
    #[serde(default = "default_node_templates_dir")]
    pub node_templates_dir: String,
    #[serde(default)]
    pub frontend: ServerFrontendConfig,
    #[serde(default)]
    pub database: ServerDatabaseConfig,
    #[serde(default)]
    pub cleanup: ServerCleanupConfig,
    #[serde(default)]
    pub auth: Option<ServerAuthConfig>,
}

fn default_run_templates_dir() -> String {
    "templates/runs".to_string()
}

fn default_task_templates_dir() -> String {
    "templates/tasks".to_string()
}

fn default_node_templates_dir() -> String {
    "templates/nodes".to_string()
}

fn default_api_host() -> IpAddr {
    "127.0.0.1".parse().expect("valid default API host")
}

fn default_server_name() -> String {
    "local".to_string()
}

fn default_api_port() -> u16 {
    4000
}

fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://localhost:8080".to_string(),
        "http://127.0.0.1:8080".to_string(),
    ]
}

fn default_allow_local_node_spawn() -> bool {
    true
}

fn default_frontend_build_dir() -> String {
    "../../../dashboard/build".to_string()
}

fn default_frontend_host() -> String {
    "127.0.0.1".to_string()
}

fn default_frontend_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerAuthConfig {
    pub admin_password_hash: String,
    pub session_secret: String,
    #[serde(default = "default_session_version")]
    pub session_version: String,
}

fn default_session_version() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerFrontendConfig {
    #[serde(default = "default_frontend_build_dir")]
    pub build_dir: String,
    #[serde(default = "default_frontend_host")]
    pub host: String,
    #[serde(default = "default_frontend_port")]
    pub port: u16,
    #[serde(default)]
    pub advertise_hosts: Vec<String>,
    #[serde(default)]
    pub access_log: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerDatabaseConfig {
    #[serde(default = "default_database_ensure_started")]
    pub ensure_started: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerCleanupConfig {
    #[serde(default = "default_sampler_drain_timeout_seconds")]
    pub sampler_drain_timeout_seconds: u64,
    #[serde(default = "default_node_stop_timeout_seconds")]
    pub node_stop_timeout_seconds: u64,
    #[serde(default = "default_cleanup_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl Default for ServerFrontendConfig {
    fn default() -> Self {
        Self {
            build_dir: default_frontend_build_dir(),
            host: default_frontend_host(),
            port: default_frontend_port(),
            advertise_hosts: Vec::new(),
            access_log: false,
        }
    }
}

impl Default for ServerDatabaseConfig {
    fn default() -> Self {
        Self {
            ensure_started: default_database_ensure_started(),
        }
    }
}

impl Default for ServerCleanupConfig {
    fn default() -> Self {
        Self {
            sampler_drain_timeout_seconds: default_sampler_drain_timeout_seconds(),
            node_stop_timeout_seconds: default_node_stop_timeout_seconds(),
            poll_interval_ms: default_cleanup_poll_interval_ms(),
        }
    }
}

fn default_database_ensure_started() -> bool {
    true
}

fn default_sampler_drain_timeout_seconds() -> u64 {
    60
}

fn default_node_stop_timeout_seconds() -> u64 {
    15
}

fn default_cleanup_poll_interval_ms() -> u64 {
    250
}

impl ServerConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let uses_embedded_default = path == Path::new(DEFAULT_SERVER_CONFIG_PATH) && !path.exists();
        let raw = read_toml_with_default_fallback(
            path,
            DEFAULT_SERVER_CONFIG_PATH,
            DEFAULT_SERVER_CONFIG_TOML,
            "server config",
        )?;
        let mut parsed: Self = toml::from_str(&raw)
            .with_context(|| format!("failed parsing server config {}", path.display()))?;
        parsed.server_config_path = path.to_path_buf();
        parsed.run_templates_dir = normalize_templates_dir(parsed.run_templates_dir.as_str())?
            .display()
            .to_string();
        parsed.task_templates_dir = normalize_templates_dir(parsed.task_templates_dir.as_str())?
            .display()
            .to_string();
        parsed.node_templates_dir = normalize_templates_dir(parsed.node_templates_dir.as_str())?
            .display()
            .to_string();
        let base_dir = if uses_embedded_default {
            std::env::current_dir().context("failed resolving current working directory")?
        } else {
            config_base_dir(path)?
        };
        parsed.frontend.build_dir = normalize_config_path(&base_dir, &parsed.frontend.build_dir)
            .display()
            .to_string();
        Ok(parsed)
    }

    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.api_host, self.api_port)
    }

    pub fn advertised_urls(&self, port: u16) -> Vec<String> {
        let hosts = if self.frontend.advertise_hosts.is_empty() {
            vec![default_advertise_host(&self.frontend.host)]
        } else {
            self.frontend.advertise_hosts.clone()
        };
        hosts
            .into_iter()
            .map(|host| format!("http://{host}:{port}"))
            .collect()
    }

    pub fn security_warnings(&self) -> Vec<String> {
        let remotely_accessible =
            !self.api_host.is_loopback() || !host_is_loopback(&self.frontend.host);
        if !remotely_accessible {
            return Vec::new();
        }

        let mut warnings = Vec::new();
        if self.auth.is_none() {
            warnings.push(
                "dashboard authentication is disabled on a non-loopback deployment; anyone who can reach the dashboard can perform administrative actions"
                    .to_string(),
            );
        } else if !self.secure_cookie {
            warnings.push(
                "dashboard authentication uses a non-secure session cookie on a non-loopback deployment; use HTTPS and set secure_cookie = true to protect the cookie in transit"
                    .to_string(),
            );
        }
        if let Some(auth) = &self.auth {
            if auth.session_secret.len() < 32 {
                warnings.push(
                    "auth.session_secret is shorter than 32 bytes; use a high-entropy secret kept outside version control"
                        .to_string(),
                );
            }
            if auth.session_secret.contains("replace-me")
                || auth.session_secret.contains("placeholder")
            {
                warnings.push(
                    "auth.session_secret appears to be a placeholder; configured authentication can be forged until it is replaced"
                        .to_string(),
                );
            }
        }
        warnings
    }
}

fn host_is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn default_advertise_host(host: &str) -> String {
    if host == "0.0.0.0" {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}

fn normalize_templates_dir(path: &str) -> anyhow::Result<PathBuf> {
    let template_dir = PathBuf::from(path);
    if template_dir.is_absolute() {
        return Ok(template_dir);
    }
    let resources_root = primary_resource_root()
        .context("failed resolving primary resource root for template directories")?;
    Ok(resources_root.join(template_dir))
}

pub async fn serve(
    store: PgStore,
    config: ServerConfig,
    runtime: RuntimeContext,
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
        store: store.clone(),
        auth: config
            .auth
            .as_ref()
            .map(|auth| AuthConfig::from_server_config(auth, &config.name)),
        server_name: config.name.clone(),
        allowed_origins,
        secure_cookie: config.secure_cookie,
        allow_local_node_spawn: config.allow_local_node_spawn,
        api_bind: bind.to_string(),
        server_config_path: config.server_config_path.clone(),
        run_templates_dir: PathBuf::from(&config.run_templates_dir),
        task_templates_dir: PathBuf::from(&config.task_templates_dir),
        node_templates_dir: PathBuf::from(&config.node_templates_dir),
        runtime: runtime.clone(),
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
    pub(crate) auth: Option<AuthConfig>,
    server_name: String,
    allowed_origins: Vec<axum::http::HeaderValue>,
    secure_cookie: bool,
    allow_local_node_spawn: bool,
    api_bind: String,
    server_config_path: PathBuf,
    run_templates_dir: PathBuf,
    task_templates_dir: PathBuf,
    node_templates_dir: PathBuf,
    runtime: RuntimeContext,
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
    #[serde(default)]
    include_children: bool,
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

const MAX_DEBUG_BATCH_LIMIT: usize = 100;

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
struct RunsQuery {
    #[serde(default)]
    include_children: bool,
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Serialize)]
struct RunPage<T> {
    items: Vec<T>,
    next_offset: Option<usize>,
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
    #[serde(default)]
    toml: Option<String>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    max_start_failures: Option<u32>,
    #[serde(default = "empty_json_object")]
    args: JsonValue,
    #[serde(default)]
    name_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLaunchToml {
    groups: Vec<NodeLaunchTomlGroup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLaunchTomlGroup {
    count: usize,
    #[serde(default)]
    name_prefix: Option<String>,
    #[serde(default = "default_node_launch_max_start_failures")]
    max_start_failures: u32,
    #[serde(default = "empty_json_object")]
    config: JsonValue,
}

fn default_node_launch_max_start_failures() -> u32 {
    3
}

fn empty_json_object() -> JsonValue {
    JsonValue::Object(Default::default())
}

#[derive(Debug, Clone)]
struct ResolvedNodeLaunchGroup {
    count: usize,
    name_prefix: String,
    max_start_failures: u32,
    config: JsonValue,
    capabilities: BTreeMap<String, u64>,
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
        .route("/auth/logout", post(logout));

    let protected_api_routes = Router::new()
        .route("/settings", get(settings::get_settings_overview))
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
        .route("/templates/nodes", get(list_node_templates))
        .route("/templates/nodes/:name", get(get_node_template))
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
        .route("/histogram-bundle/export", post(export_histogram_bundle))
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
        .route("/nodes/unassign-all", post(unassign_all_nodes))
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
        .route("/templates/nodes", post(save_node_template))
        .route("/templates/runs/:name", delete(delete_run_template))
        .route("/templates/tasks/:name", delete(delete_task_template))
        .route("/templates/nodes/:name", delete(delete_node_template))
        .route("/admin/db/restart", post(restart_db))
        .route("/admin/control/shutdown", post(shutdown_control_process))
        .route("/runs/:id/debug/batches", get(get_run_debug_batches))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_session,
        ));

    Router::new()
        .nest("/api", public_api_routes.merge(protected_api_routes))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
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
                "database": "connected",
                "server_name": state.server_name,
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "database": "disconnected",
                "server_name": state.server_name,
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
    Query(params): Query<RunsQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let offset = params.offset.unwrap_or(0);
    let runs = state
        .store
        .get_runs_page(limit, offset, params.include_children)
        .await?;
    let next_offset = (runs.len() == limit).then_some(offset + runs.len());
    json_response(RunPage {
        items: runs,
        next_offset,
    })
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
        .get_registered_worker(&node_name)
        .await?
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
        .unwrap_or(MAX_DEBUG_BATCH_LIMIT)
        .clamp(1, MAX_DEBUG_BATCH_LIMIT);
    let status_param = params
        .get("status")
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "claimed".to_string());
    let status_param = match status_param.as_str() {
        "pending" | "claimed" | "failed" | "all" => status_param,
        _ => "claimed".to_string(),
    };
    let payload = state
        .store
        .fetch_pending_claimed_batches_json(run_id, status_param.as_str(), limit)
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

async fn list_node_templates(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: template_api::list_templates(&state.node_templates_dir)?,
    })
}

async fn get_node_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template = template_api::load_template(&state.node_templates_dir, &name)?;
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

async fn save_node_template(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<TemplateSaveRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template =
        template_api::save_template(&state.node_templates_dir, &payload.name, &payload.toml)?;
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

async fn delete_node_template(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    template_api::delete_template(&state.node_templates_dir, &name)?;
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
    let Some(evaluator) = run_spec.integration_params.evaluator.as_ref() else {
        return Err(ApiError::NotFound(format!(
            "run {run_id} has no root evaluator config"
        )));
    };
    let response: PanelResponse = evaluator
        .build_response(
            format!("run:{run_id}:config:evaluator"),
            &EvaluatorPanelContext {
                domain: &run_spec.domain,
                runner_params: &run_spec.integration_params.evaluator_runner_params,
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
                runner_params: &run_spec.integration_params.sampler_aggregator_runner_params,
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
        && let Some(config) = source_snapshot.sampler_aggregator
    {
        return Ok(config);
    }

    if let Some(base_snapshot) = state
        .store
        .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
        .await?
        && let Some(config) = base_snapshot.sampler_aggregator
    {
        return Ok(config);
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
    if !matches!(task.state, crate::core::RunTaskState::Active)
        && request.request.panel_actions.is_empty()
        && cursor.snapshot_id.is_some()
        && latest_persisted_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.id.parse::<i64>().ok())
            == cursor.snapshot_id
    {
        return json_response(crate::server::panels::PanelResponse {
            source_id: format!("run:{run_id}:task:{}", task.id),
            cursor: request.request.cursor.clone(),
            reset_required: false,
            panels: Vec::new(),
            updates: Vec::new(),
            poll_after_ms: None,
        });
    }
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
            .get_task_output_snapshots(run_id, task.id, None, limit)
            .await?
    } else {
        Vec::new()
    };
    let latest_sampler_performance = if matches!(task.task, crate::core::RunTaskSpec::Sample { .. })
    {
        state
            .store
            .get_sampler_performance_history(run_id, 1, None)
            .await?
            .into_iter()
            .next()
    } else {
        None
    };
    let sampler_engine_diagnostics = latest_sampler_performance
        .as_ref()
        .map(|entry| entry.engine_diagnostics.clone());
    let (completed_samples_per_second, smoothed_eta_seconds) =
        if matches!(task.task, crate::core::RunTaskSpec::Sample { .. })
            && matches!(task.state, crate::core::RunTaskState::Active)
        {
            let metrics = latest_sampler_performance.as_ref().and_then(|entry| {
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
            sampler_engine_diagnostics.as_ref(),
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
            params.include_children,
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
            params.include_children,
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
    let config = run_api::parse_run_add_config_toml(&payload.toml).map_err(|err| {
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

async fn unassign_all_nodes(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows_updated = node_api::unassign_all_nodes(&state.store)
        .await
        .map_err(|err| {
            log_control_api_error("node_unassign_all", &err);
            err
        })?;
    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "node_unassign_all",
        rows_updated,
        "dashboard action completed"
    );
    json_response(serde_json::json!({
        "rows_updated": rows_updated,
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

#[derive(Debug, Clone)]
struct PlannedNodeStart {
    node_name: String,
    max_start_failures: u32,
    capabilities: BTreeMap<String, u64>,
}

fn derive_capabilities_from_config(config: &JsonValue) -> BTreeMap<String, u64> {
    let mut caps = BTreeMap::new();
    let Some(map) = config.as_object() else {
        return caps;
    };
    for (key, value) in map {
        if key == "gpu" {
            if let Some(count) = gpu_count_from_config_value(value) {
                caps.insert("gpu".to_string(), count);
            }
            continue;
        }
        if matches!(
            key.as_str(),
            "cores" | "nr_cores" | "cpus" | "cpus_per_task" | "cpus-per-task"
        ) {
            if let Some(number) = value.as_u64() {
                caps.insert("cpus".to_string(), number);
            }
            continue;
        }
        if let Some(number) = value.as_u64() {
            caps.insert(key.clone(), number);
        }
    }
    if let Some(gres) = map.get("gres").and_then(JsonValue::as_str)
        && let Some(count) = parse_gpu_count_from_gres(gres)
    {
        caps.insert("gpu".to_string(), count);
    }
    caps
}

fn gpu_count_from_config_value(value: &JsonValue) -> Option<u64> {
    if let Some(count) = value.as_u64() {
        return (count > 0).then_some(count);
    }
    let raw = value.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(count) = raw.parse::<u64>() {
        return (count > 0).then_some(count);
    }
    parse_gpu_count_from_gres(if raw.starts_with("gpu:") {
        raw
    } else {
        return parse_gpu_count_from_gres(&format!("gpu:{raw}"));
    })
}

fn parse_gpu_count_from_gres(gres: &str) -> Option<u64> {
    for segment in gres.split(',') {
        let trimmed = segment.trim();
        if !trimmed.starts_with("gpu:") {
            continue;
        }
        let parts = trimmed.split(':').collect::<Vec<_>>();
        let last = parts.last().copied().unwrap_or_default();
        if let Ok(count) = last.parse::<u64>() {
            return Some(count);
        }
        return Some(1);
    }
    None
}

fn resolve_node_launch_groups(
    payload: &AutoRunNodesRequest,
) -> Result<Vec<ResolvedNodeLaunchGroup>, ApiError> {
    if let Some(toml_text) = payload.toml.as_ref() {
        let parsed: NodeLaunchToml =
            toml_template::parse_templated_toml(toml_text, "node launch TOML")?;
        if parsed.groups.is_empty() {
            return Err(ApiError::BadRequest(
                "node launch TOML requires at least one [[groups]] entry".to_string(),
            ));
        }
        let groups = parsed
            .groups
            .into_iter()
            .enumerate()
            .map(|(index, group)| {
                if group.count == 0 {
                    return Err(ApiError::BadRequest(format!(
                        "groups[{index}].count must be greater than zero"
                    )));
                }
                let name_prefix = group
                    .name_prefix
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("w")
                    .to_string();
                let capabilities = derive_capabilities_from_config(&group.config);
                Ok(ResolvedNodeLaunchGroup {
                    count: group.count,
                    name_prefix,
                    max_start_failures: group.max_start_failures,
                    config: group.config,
                    capabilities,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(groups);
    }

    let count = payload.count.ok_or_else(|| {
        ApiError::BadRequest("count is required when no launch TOML is provided".to_string())
    })?;
    if count == 0 {
        return Err(ApiError::BadRequest(
            "requested node count must be greater than zero".to_string(),
        ));
    }
    Ok(vec![ResolvedNodeLaunchGroup {
        count,
        name_prefix: payload
            .name_prefix
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("w")
            .to_string(),
        max_start_failures: payload.max_start_failures.unwrap_or(3),
        config: JsonValue::Object(Default::default()),
        capabilities: BTreeMap::new(),
    }])
}

async fn plan_group_node_names(
    store: &PgStore,
    groups: &[ResolvedNodeLaunchGroup],
) -> Result<Vec<PlannedNodeStart>, ApiError> {
    let existing = store
        .list_nodes(None)
        .await?
        .into_iter()
        .map(|node| node.name)
        .collect::<HashSet<_>>();
    let mut taken = existing;
    let mut planned = Vec::new();
    for group in groups {
        let mut index = 1usize;
        let mut generated = 0usize;
        while generated < group.count {
            let candidate = format!("{}-{}", group.name_prefix, index);
            index = index.saturating_add(1);
            if taken.contains(&candidate) {
                continue;
            }
            taken.insert(candidate.clone());
            planned.push(PlannedNodeStart {
                node_name: candidate,
                max_start_failures: group.max_start_failures,
                capabilities: group.capabilities.clone(),
            });
            generated += 1;
        }
    }
    Ok(planned)
}

async fn create_and_maybe_resolve_node_launch_request(
    state: AppState,
    payload: AutoRunNodesRequest,
) -> Result<Json<serde_json::Value>, ApiError> {
    let groups = resolve_node_launch_groups(&payload)?;
    let requested_count = groups.iter().map(|group| group.count).sum::<usize>();
    let backend = if state.allow_local_node_spawn {
        "local"
    } else {
        "external"
    };
    let mut args = serde_json::Map::new();
    if let JsonValue::Object(map) = payload.args.clone() {
        args.extend(map);
    }
    let groups_json = groups
        .iter()
        .map(|group| {
            serde_json::json!({
                "count": group.count,
                "name_prefix": group.name_prefix,
                "max_start_failures": group.max_start_failures,
                "config": group.config,
            })
        })
        .collect::<Vec<_>>();
    args.insert("groups".to_string(), JsonValue::Array(groups_json));
    if let Some(toml) = payload.toml.as_ref() {
        args.insert("toml".to_string(), JsonValue::String(toml.clone()));
    }
    let launch = node_api::create_node_launch_request(
        &state.store,
        requested_count,
        backend,
        None,
        &JsonValue::Object(args),
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
            "requested": requested_count,
            "started": 0,
            "node_names": [],
        }));
    }

    let planned_nodes = plan_group_node_names(&state.store, &groups)
        .await
        .map_err(|err| {
            log_control_api_error("node_auto_run", &err);
            err
        })?;

    let binary = std::env::current_exe().map_err(|err| {
        ApiError::Internal(format!("failed to resolve current executable: {err}"))
    })?;

    let runtime_cli_args = state.runtime.runtime_cli_args();
    let mut started_node_names = Vec::new();
    for planned in &planned_nodes {
        if let Err(err) = spawn_node_process(
            &binary,
            &runtime_cli_args,
            &state.runtime,
            &planned.node_name,
            planned.max_start_failures,
            &planned.capabilities,
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
        started_node_names.push(planned.node_name.clone());
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
        requested = requested_count,
        started = started_node_names.len(),
        node_names = ?started_node_names,
        "dashboard action completed"
    );

    json_response(serde_json::json!({
        "request": request,
        "requested": requested_count,
        "started": started_node_names.len(),
        "node_names": started_node_names,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_warnings_keep_passwordless_loopback_deployments_quiet() {
        let mut config: ServerConfig =
            toml::from_str(DEFAULT_SERVER_CONFIG_TOML).expect("default server config");
        config.server_config_path = PathBuf::new();
        assert!(config.security_warnings().is_empty());
    }

    #[test]
    fn security_warnings_report_insecure_remote_deployments() {
        let mut config: ServerConfig =
            toml::from_str(DEFAULT_SERVER_CONFIG_TOML).expect("default server config");
        config.server_config_path = PathBuf::new();
        config.frontend.host = "0.0.0.0".to_string();

        let warnings = config.security_warnings();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("authentication is disabled"))
        );

        config.auth = Some(ServerAuthConfig {
            admin_password_hash: "public-development-placeholder".to_string(),
            session_secret: "public-development-placeholder".to_string(),
            session_version: "1".to_string(),
        });
        let warnings = config.security_warnings();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("non-secure session cookie"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("shorter than 32 bytes"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("appears to be a placeholder"))
        );
    }

    #[test]
    fn resolve_node_launch_groups_expands_typed_top_level_replacements() {
        let payload = AutoRunNodesRequest {
            toml: Some(
                r#"
replacements = { count = 2, prefix = "cpu", cores = 4 }

[[groups]]
count = "$(count:1)"
name_prefix = '$(prefix:"worker")'
config = { cores = "$(cores:1)" }
"#
                .to_string(),
            ),
            count: None,
            max_start_failures: None,
            args: empty_json_object(),
            name_prefix: None,
        };

        let groups = resolve_node_launch_groups(&payload).expect("node launch groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].name_prefix, "cpu");
        assert_eq!(groups[0].capabilities.get("cpus"), Some(&4));
    }

    #[test]
    fn node_launch_group_cpu_aliases_derive_cpus_capability() {
        for key in [
            "cores",
            "nr_cores",
            "cpus",
            "cpus_per_task",
            "cpus-per-task",
        ] {
            let config = serde_json::json!({ key: 8 });
            let capabilities = derive_capabilities_from_config(&config);
            assert_eq!(
                capabilities.get("cpus"),
                Some(&8),
                "cpu alias {key} should register cpus capability"
            );
            if key != "cpus" {
                assert_eq!(capabilities.get(key), None);
            }
        }
    }

    #[test]
    fn bundled_node_templates_parse_after_replacement_expansion() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for relative_path in [
            "resources/templates/nodes/local-two-workers.toml",
            "ops/ubelix/resources/templates/nodes/cpu-workers.toml",
            "ops/ubelix/resources/templates/nodes/free-gpu-plus-cpu.toml",
        ] {
            let toml = std::fs::read_to_string(root.join(relative_path)).expect("template file");
            let payload = AutoRunNodesRequest {
                toml: Some(toml),
                count: None,
                max_start_failures: None,
                args: empty_json_object(),
                name_prefix: None,
            };
            resolve_node_launch_groups(&payload)
                .unwrap_or_else(|err| panic!("{relative_path} should parse: {err}"));
        }
    }

    #[test]
    fn server_config_does_not_expand_placeholders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("server.toml");
        std::fs::write(&path, r#"name = '$(server_name:"local")'"#).expect("write server config");

        let config = ServerConfig::load(&path).expect("server config");
        assert_eq!(config.name, r#"$(server_name:"local")"#);
    }
}

async fn restart_db(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let binary = std::env::current_exe().map_err(|err| {
        ApiError::Internal(format!("failed to resolve current executable: {err}"))
    })?;
    let runtime_cli_args = state.runtime.runtime_cli_args();
    let result = db_api::restart_local_database(&binary, &runtime_cli_args).map_err(|err| {
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
    runtime: &RuntimeContext,
    node_name: &str,
    max_start_failures: u32,
    capabilities: &BTreeMap<String, u64>,
) -> Result<(), ApiError> {
    use std::process::Stdio;
    use tokio::process::Command;

    let (stdout_log_path, stderr_log_path) = node_process_log_paths(runtime, node_name)?;
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
        .args(node_api::node_run_cli_args(
            node_name,
            max_start_failures,
            capabilities,
        ))
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

fn node_process_log_paths(
    runtime: &RuntimeContext,
    node_name: &str,
) -> Result<(PathBuf, PathBuf), ApiError> {
    runtime
        .node_log_paths(node_name)
        .map_err(|err| ApiError::Internal(format!("failed resolving node log paths: {err}")))
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
    let response = match format.as_str() {
        "json" => {
            let contents = serde_json::to_string_pretty(&wrapper).map_err(|err| {
                ApiError::Internal(format!("failed to serialize histogram bundle json: {err}"))
            })?;
            HistogramBundleExportResponse {
                filename: "histogram_bundle.json".to_string(),
                mime_type: "application/json;charset=utf-8".to_string(),
                contents: format!("{contents}\n"),
            }
        }
        "hwu" => {
            #[cfg(not(feature = "gammaloop"))]
            {
                return Err(ApiError::BadRequest(
                    "HwU histogram bundle export requires a gammaboard build with the default \"gammaloop\" feature enabled"
                        .to_string(),
                ));
            }
            #[cfg(feature = "gammaloop")]
            {
                let bundle: ObservableSnapshotBundle =
                    serde_json::from_value(wrapper).map_err(|err| {
                        ApiError::BadRequest(format!(
                            "invalid histogram bundle payload for export: {err}"
                        ))
                    })?;
                let file = tempfile::NamedTempFile::new().map_err(|err| {
                    ApiError::Internal(format!("failed to create temporary hwu file: {err}"))
                })?;
                bundle.write_hwu_file(file.path()).map_err(|err| {
                    ApiError::Internal(format!("failed to export HwU bundle: {err}"))
                })?;
                let contents = fs::read_to_string(file.path()).map_err(|err| {
                    ApiError::Internal(format!("failed to read exported HwU bundle: {err}"))
                })?;
                HistogramBundleExportResponse {
                    filename: "histogram_bundle.HwU".to_string(),
                    mime_type: "text/plain;charset=utf-8".to_string(),
                    contents,
                }
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
