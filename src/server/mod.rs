use crate::api::node_launch::AutoRunNodesRequest;
#[cfg(test)]
use crate::api::node_launch::{derive_capabilities_from_config, resolve_node_launch_groups};
mod auth;
mod config_panels;
mod panels;
mod performance_panels;
mod routes;
mod run_panels;
mod settings;
mod task_panels;
mod worker_panels;

use crate::api::{ApiError, nodes as node_api, runs as run_api, templates as template_api};
use crate::core::{
    AggregationStore, EngineError, RunReadStore, RunSpec, RunSpecStore, RunTask, RunTaskSpec,
    RunTaskState, RunTaskStore, SamplerQueueTuning,
};
use crate::evaluation::AccumulatorState;
use crate::runners::stage_context::{StageConfigProvenance, resolve_stage_context};
use crate::server::config_panels::{
    EvaluatorPanelContext, PanelProvider, SamplerAggregatorPanelContext,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelRequest, PanelResponse, PanelWidth, replace_panel,
    sized_panel_spec, text_panel,
};
use crate::server::performance_panels::{
    build_evaluator_performance_response, build_sampler_performance_response,
};
use crate::server::run_panels::build_run_panel_response;
use crate::server::task_panels::{TaskPanelSource, parse_cursor as parse_task_panel_cursor};
use crate::server::worker_panels::build_worker_panel_response;
use crate::stores::{PgStore, RunLifecycleState, RunProgress};
use anyhow::Context;
use axum::{
    extract::{Json as AxumJson, Path as AxumPath, Query, State},
    http::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
};
#[cfg(feature = "gammaloop")]
use gammalooprs::observables::ObservableSnapshotBundle;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
#[cfg(feature = "gammaloop")]
use std::fs;
use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};
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

    let app = routes::build_app(state);

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
            ApiError::Forbidden(message) => (StatusCode::FORBIDDEN, message),
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
        ApiError::Unauthorized(message) | ApiError::Forbidden(message) => tracing::warn!(
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

#[derive(Serialize)]
struct RunSummaryResponse {
    run_id: i32,
    run_name: String,
    parent_run_id: Option<i32>,
    spawn_label: Option<String>,
    root_stage_snapshot_id: Option<String>,
    lifecycle_state: RunLifecycleState,
    nr_completed_samples_including_children: i64,
    cpu_hours_including_children: f64,
    queue_tuning_defaults: Option<JsonValue>,
}

impl From<RunProgress> for RunSummaryResponse {
    fn from(run: RunProgress) -> Self {
        let queue_tuning_defaults = run.integration_params.as_ref().and_then(|params| {
            params
                .pointer("/sampler_aggregator_runner_params/queue")
                .cloned()
        });
        Self {
            run_id: run.run_id,
            run_name: run.run_name,
            parent_run_id: run.parent_run_id,
            spawn_label: run.spawn_label,
            root_stage_snapshot_id: run.root_stage_snapshot_id,
            lifecycle_state: run.lifecycle_state,
            nr_completed_samples_including_children: run.nr_completed_samples_including_children,
            cpu_hours_including_children: run.cpu_seconds_including_children / 3600.0,
            queue_tuning_defaults,
        }
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
struct NodeLaunchRequestProgressRequest {
    state: String,
    started_count: usize,
    #[serde(default)]
    result: JsonValue,
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
    queue_tuning: Option<SamplerQueueTuning>,
}

#[derive(Deserialize)]
struct TemplateSaveRequest {
    name: String,
    toml: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TemplateKind {
    Runs,
    Tasks,
    Nodes,
}

#[derive(Serialize)]
struct TemplateListResponse {
    items: Vec<String>,
}

#[derive(Serialize)]
struct RunReproTomlResponse {
    toml: String,
}

#[derive(Serialize)]
struct RunTaskResponse {
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string")]
    id: i64,
    run_id: i32,
    name: String,
    sequence_nr: i32,
    task_kind: String,
    goal_label: String,
    is_sample: bool,
    queue_tuning: Option<SamplerQueueTuning>,
    state: RunTaskState,
    nr_completed_samples_including_children: i64,
    cpu_hours_including_children: f64,
    failure_reason: Option<String>,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    latest_stage_snapshot_id: Option<i64>,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    root_stage_snapshot_id: Option<i64>,
}

impl RunTaskResponse {
    fn new(
        task: RunTask,
        latest_stage_snapshot_id: Option<i64>,
        root_stage_snapshot_id: Option<i64>,
    ) -> Self {
        let task_kind = task.task.kind_str().to_string();
        let goal_label = match &task.task {
            RunTaskSpec::SetAccumulator { .. } => "-".to_string(),
            task => task
                .nr_expected_samples()
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unbounded".to_string()),
        };
        let queue_tuning = match &task.task {
            RunTaskSpec::Sample { queue_tuning, .. } => queue_tuning.clone(),
            _ => None,
        };
        let is_sample = matches!(task.task, RunTaskSpec::Sample { .. });
        Self {
            id: task.id,
            run_id: task.run_id,
            name: task.name,
            sequence_nr: task.sequence_nr,
            task_kind,
            goal_label,
            is_sample,
            queue_tuning,
            state: task.state,
            nr_completed_samples_including_children: task.nr_completed_samples_including_children,
            cpu_hours_including_children: task.cpu_seconds_including_children / 3600.0,
            failure_reason: task.failure_reason,
            latest_stage_snapshot_id,
            root_stage_snapshot_id,
        }
    }
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
) -> Result<Json<SessionStatus>, ApiError> {
    auth::validate_origin(&headers, &state.allowed_origins)?;
    Ok(Json(auth::auth_status_from_headers(&state, &headers)))
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
        items: runs.into_iter().map(RunSummaryResponse::from).collect(),
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

async fn get_run_repro_toml(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let toml = run_api::export_run_repro_toml(&state.store, id).await?;
    json_response(RunReproTomlResponse { toml })
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
    let mut response = build_run_panel_response(&run, &run_spec, &tasks, &workers)
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    let checkpoint = state.store.checkpoint_status(run_id).await?;
    crate::server::run_panels::append_checkpoint_panel(&mut response, &checkpoint);

    let active_task = tasks.iter().find(|task| task.state.as_str() == "active");
    let configs = match active_task {
        Some(task) if task.task.runs_on_sampler_worker() => {
            resolve_stage_context(&state.store, run_id, task, task.sequence_nr, None)
                .await
                .map(|resolved| {
                    (
                        Some((resolved.evaluator_config, resolved.evaluator_provenance)),
                        Some((resolved.sampler_config, resolved.sampler_provenance)),
                    )
                })
        }
        Some(task) if task.task.is_controller() => Ok((None, None)),
        _ => state
            .store
            .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
            .await
            .map(|snapshot| configs_from_stage_snapshot(snapshot.as_ref())),
    };

    match configs {
        Ok((evaluator, sampler)) => {
            append_engine_config_panels(&mut response, &run_spec, evaluator, sampler)
                .map_err(|err| ApiError::Internal(err.to_string()))?;
        }
        Err(err) => {
            tracing::warn!(run_id, error = %err, "failed to resolve run engine configuration panels");
            response.panels.push(sized_panel_spec(
                "engine_config_error",
                "Engine Configuration",
                PanelKind::Text,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ));
            response.updates.push(replace_panel(text_panel(
                "engine_config_error",
                format!("Failed to resolve the effective engine configuration: {err}"),
            )));
        }
    }

    json_response(response)
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
        .map(|task| {
            let latest_stage_snapshot_id = latest_snapshot_ids.get(&task.id).copied();
            RunTaskResponse::new(task, latest_stage_snapshot_id, root_stage_snapshot_id)
        })
        .collect::<Vec<_>>();
    json_response(response)
}

fn template_dir(state: &AppState, kind: TemplateKind) -> &Path {
    match kind {
        TemplateKind::Runs => &state.run_templates_dir,
        TemplateKind::Tasks => &state.task_templates_dir,
        TemplateKind::Nodes => &state.node_templates_dir,
    }
}

async fn list_templates(
    State(state): State<AppState>,
    AxumPath(kind): AxumPath<TemplateKind>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    json_response(TemplateListResponse {
        items: template_api::list_templates(template_dir(&state, kind))?,
    })
}

async fn get_template(
    State(state): State<AppState>,
    AxumPath((kind, name)): AxumPath<(TemplateKind, String)>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template = template_api::load_template(template_dir(&state, kind), &name)?;
    json_response(template)
}

async fn save_template(
    State(state): State<AppState>,
    AxumPath(kind): AxumPath<TemplateKind>,
    AxumJson(payload): AxumJson<TemplateSaveRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let template =
        template_api::save_template(template_dir(&state, kind), &payload.name, &payload.toml)?;
    json_response(template)
}

async fn delete_template(
    State(state): State<AppState>,
    AxumPath((kind, name)): AxumPath<(TemplateKind, String)>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    template_api::delete_template(template_dir(&state, kind), &name)?;
    json_response(serde_json::json!({ "deleted": true, "name": name }))
}

fn append_engine_config_panels(
    response: &mut PanelResponse,
    run_spec: &RunSpec,
    evaluator: Option<EffectiveEvaluatorConfig>,
    sampler: Option<EffectiveSamplerConfig>,
) -> Result<(), EngineError> {
    if let Some((evaluator, provenance)) = evaluator {
        let provenance = format_stage_config_provenance(&provenance);
        let context = EvaluatorPanelContext {
            domain: &run_spec.domain,
            runner_params: &run_spec.integration_params.evaluator_runner_params,
            provenance: &provenance,
        };
        response.panels.extend(evaluator.panel_specs(&context));
        response.updates.extend(
            evaluator
                .panel_states(&context)?
                .into_iter()
                .map(replace_panel),
        );
    }
    if let Some((sampler, provenance)) = sampler {
        let provenance = format_stage_config_provenance(&provenance);
        let context = SamplerAggregatorPanelContext {
            domain: &run_spec.domain,
            runner_params: &run_spec.integration_params.sampler_aggregator_runner_params,
            provenance: &provenance,
        };
        response.panels.extend(sampler.panel_specs(&context));
        response.updates.extend(
            sampler
                .panel_states(&context)?
                .into_iter()
                .map(replace_panel),
        );
    }
    Ok(())
}

type EffectiveEvaluatorConfig = (crate::core::EvaluatorConfig, StageConfigProvenance);
type EffectiveSamplerConfig = (crate::core::SamplerAggregatorConfig, StageConfigProvenance);

fn configs_from_stage_snapshot(
    snapshot: Option<&crate::core::RunStageSnapshot>,
) -> (
    Option<EffectiveEvaluatorConfig>,
    Option<EffectiveSamplerConfig>,
) {
    let evaluator = snapshot.and_then(|snapshot| {
        snapshot
            .evaluator
            .clone()
            .map(|config| (config, StageConfigProvenance::from_snapshot(snapshot)))
    });
    let sampler = snapshot.and_then(|snapshot| {
        snapshot
            .sampler_aggregator
            .clone()
            .map(|config| (config, StageConfigProvenance::from_snapshot(snapshot)))
    });
    (evaluator, sampler)
}

fn format_stage_config_provenance(provenance: &StageConfigProvenance) -> String {
    match (provenance.task_id, provenance.snapshot_id) {
        (Some(task_id), Some(snapshot_id)) => format!(
            "stage '{}' (task #{task_id}, snapshot #{snapshot_id})",
            provenance.name
        ),
        (Some(task_id), None) => format!("task '{}' (#{task_id})", provenance.name),
        (None, Some(snapshot_id)) => {
            format!("stage '{}' (snapshot #{snapshot_id})", provenance.name)
        }
        (None, None) => format!("stage '{}'", provenance.name),
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
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let effective_accumulator_config =
        if matches!(task.task, crate::core::RunTaskSpec::Sample { .. }) {
            crate::services::stage::try_resolve_effective_sample_accumulator_config(
                &state.store,
                run_id,
                &task,
            )
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
    let config = run_api::parse_run_add_config_toml(&payload.toml)
        .inspect_err(|err| log_control_api_error("run_create", err))?;
    let run = run_api::create_run(&state.store, config)
        .await
        .inspect_err(|err| log_control_api_error("run_create", err))?;
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
    .inspect_err(|err| log_control_api_error("run_clone", err))?;
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
    let tasks = run_api::parse_task_queue_toml(payload.toml.trim())
        .inspect_err(|err| log_control_api_error("run_add_tasks", err))?;
    let result = run_api::append_tasks(&state.store, run_id, tasks)
        .await
        .inspect_err(|err| log_control_api_error("run_add_tasks", err))?;
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
        .inspect_err(|err| log_control_api_error("run_pause", err))?;
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
        .inspect_err(|err| log_control_api_error("run_remove", err))?;
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
        .inspect_err(|err| log_control_api_error("run_task_remove", err))?;
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
            .inspect_err(|err| log_control_api_error("run_task_update_queue_tuning", err))?;
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
    .inspect_err(|err| log_control_api_error("node_assign", err))?;
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
        .inspect_err(|err| log_control_api_error("run_auto_assign", err))?;

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
        .inspect_err(|err| log_control_api_error("node_unassign", err))?;
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
        .inspect_err(|err| log_control_api_error("node_unassign_all", err))?;
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
        .inspect_err(|err| log_control_api_error("node_stop", err))?;
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
        .inspect_err(|err| log_control_api_error("node_stop_all", err))?;
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
    let requests = node_api::list_node_launch_requests(&state.store)
        .await
        .inspect_err(|err| log_control_api_error("node_launch_requests_list", err))?;
    json_response(serde_json::json!({ "items": requests }))
}

async fn claim_external_node_launch_request(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = node_api::claim_external_node_launch_request(&state.store)
        .await
        .inspect_err(|err| log_control_api_error("node_launch_request_claim_external", err))?;
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
    .inspect_err(|err| log_control_api_error("node_launch_request_progress", err))?;
    json_response(serde_json::json!({ "request": request }))
}

async fn create_and_maybe_resolve_node_launch_request(
    state: AppState,
    payload: AutoRunNodesRequest,
) -> Result<Json<serde_json::Value>, ApiError> {
    json_response(
        crate::api::node_launch::launch(
            &state.store,
            &state.runtime,
            state.allow_local_node_spawn,
            payload,
        )
        .await?,
    )
}

async fn restart_db(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let runtime = state.runtime.runtime_config();
    let local = runtime.local_postgres.clone();
    let database_url = runtime.database.url.clone();
    tokio::task::spawn_blocking(move || crate::local_db::reset_db(&local, true, &database_url))
        .await
        .map_err(|err| ApiError::Internal(format!("database restart task failed: {err}")))?
        .map_err(|err| ApiError::Internal(err.to_string()))
        .inspect_err(|err| log_control_api_error("db_restart", err))?;

    tracing::info!(
        source = "control",
        control_surface = "dashboard",
        action = "db_restart",
        deleted = true,
        started = true,
        "dashboard action completed"
    );

    json_response(serde_json::json!({
        "deleted": true,
        "started": true,
    }))
}

async fn suspend_workers(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let saved = state
        .store
        .suspend_workers()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    json_response(serde_json::json!({"workers_saved": saved}))
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
        let result = node_api::suspend_nodes_gracefully(
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

async fn get_run_performance(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let limit = clamp_limit(params.limit);
    let (sampler_rows, run_evaluator_rows, evaluator_rows) = tokio::try_join!(
        state.store.get_sampler_performance_history(id, limit, None),
        state
            .store
            .get_evaluator_performance_history(id, limit, None),
        async {
            match params.node_name.as_deref() {
                Some(node_name) => state
                    .store
                    .get_evaluator_performance_history(id, limit, Some(node_name))
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    )?;
    let mut response =
        build_sampler_performance_response(Some(format!("run:{id}:performance")), sampler_rows);
    let run_evaluator = build_evaluator_performance_response(None, run_evaluator_rows, true);
    response.panels.extend(run_evaluator.panels);
    response.updates.extend(run_evaluator.updates);
    if let Some(rows) = evaluator_rows {
        let evaluator = build_evaluator_performance_response(None, rows, false);
        response.panels.extend(evaluator.panels);
        response.updates.extend(evaluator.updates);
    }
    response.cursor = None;
    json_response(response)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_list_response_omits_duplicate_and_runtime_only_fields() {
        let task = RunTask {
            id: 7,
            run_id: 3,
            name: "accumulator".to_string(),
            sequence_nr: 1,
            task: RunTaskSpec::SetAccumulator {
                accumulator: crate::core::AccumulatorConfig::Empty,
            },
            spawned_from_snapshot_id: None,
            state: RunTaskState::Pending,
            nr_produced_samples: 11,
            nr_completed_samples: 9,
            nr_produced_samples_including_children: 11,
            nr_completed_samples_including_children: 9,
            cpu_seconds: 1800.0,
            cpu_seconds_including_children: 3600.0,
            failure_reason: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            created_at: chrono::Utc::now(),
            task_toml: "kind = \"set_accumulator\"".to_string(),
            measurement_output: None,
            controller_output: None,
        };
        let value = serde_json::to_value(RunTaskResponse::new(task, Some(8), Some(2))).unwrap();

        assert_eq!(value["id"], "7");
        assert_eq!(value["cpu_hours_including_children"], 1.0);
        assert_eq!(value["task_kind"], "set_accumulator");
        assert_eq!(value["goal_label"], "-");
        assert_eq!(value["is_sample"], false);
        for omitted in [
            "task",
            "task_toml",
            "nr_produced_samples",
            "nr_completed_samples",
            "cpu_seconds",
            "cpu_seconds_including_children",
            "started_at",
            "completed_at",
            "created_at",
            "measurement_output",
            "controller_output",
        ] {
            assert!(value.get(omitted).is_none(), "unexpected field {omitted}");
        }
    }

    #[test]
    fn template_kinds_are_strictly_allowlisted() {
        assert_eq!(
            serde_json::from_str::<TemplateKind>(r#""runs""#).unwrap(),
            TemplateKind::Runs
        );
        assert_eq!(
            serde_json::from_str::<TemplateKind>(r#""tasks""#).unwrap(),
            TemplateKind::Tasks
        );
        assert_eq!(
            serde_json::from_str::<TemplateKind>(r#""nodes""#).unwrap(),
            TemplateKind::Nodes
        );
        assert!(serde_json::from_str::<TemplateKind>(r#""other""#).is_err());
    }

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
            args: serde_json::json!({}),
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
                args: serde_json::json!({}),
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
