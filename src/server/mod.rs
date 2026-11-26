use crate::api::node_launch::AutoRunNodesRequest;
async fn get_run_metrics(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if state.store.get_run_progress(id).await?.is_none() {
        return Err(ApiError::NotFound(format!("run {id}")));
    }
    let snapshot = crate::api::performance::snapshot(&state.store, id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    json_response(
        serde_json::to_value(snapshot).map_err(|err| ApiError::Internal(err.to_string()))?,
    )
}

#[cfg(test)]
use crate::api::node_launch::resolve_node_launch_groups;
mod auth;
mod campaign;
mod config_panels;
mod histogram_export;
mod histogram_import;
pub mod manage;
pub mod origins;
mod panels;
mod performance_panels;
mod routes;
mod run_panels;
mod run_removals;
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
    routing::{get, post},
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    #[serde(default = "default_allow_loopback_origins")]
    pub allow_loopback_origins: bool,
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
    pub cleanup: node_api::GracefulNodeShutdownParams,
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

fn default_allow_loopback_origins() -> bool {
    true
}

fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://localhost:4000".to_string(),
        "http://127.0.0.1:4000".to_string(),
    ]
}

fn default_allow_local_node_spawn() -> bool {
    true
}

fn default_frontend_build_dir() -> String {
    "../../../dashboard/build".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerFrontendConfig {
    #[serde(default = "default_frontend_build_dir")]
    pub build_dir: String,
}
impl Default for ServerFrontendConfig {
    fn default() -> Self {
        Self {
            build_dir: default_frontend_build_dir(),
        }
    }
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

    pub fn security_warnings(&self) -> Vec<String> {
        let remotely_accessible = !self.api_host.is_loopback();
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

fn normalize_templates_dir(path: &str) -> anyhow::Result<PathBuf> {
    let template_dir = PathBuf::from(path);
    if template_dir.is_absolute() {
        return Ok(template_dir);
    }
    let resources_root = primary_resource_root()
        .context("failed resolving primary resource root for template directories")?;
    Ok(resources_root.join(template_dir))
}

/// A local deployment owner already holds the signing secret in its private
/// configuration. This avoids storing a second bootstrap credential or IPC socket.
pub fn bootstrap_token(config: &ServerConfig) -> anyhow::Result<Option<String>> {
    config
        .auth
        .as_ref()
        .map(|auth| {
            auth::sign_session_token(&AuthConfig::from_server_config(auth, &config.name))
                .map_err(Into::into)
        })
        .transpose()
}

pub async fn serve(
    store: PgStore,
    config: ServerConfig,
    runtime: RuntimeContext,
) -> anyhow::Result<()> {
    let origins = origins::BrowserOrigins::new(&config)?;
    spawn(store, config, runtime, false, None, origins)
        .await?
        .await??;
    Ok(())
}

/// Start the API and optional static dashboard in the owning service process.
/// Binding completes before this returns, so callers can report readiness safely.
pub async fn spawn(
    store: PgStore,
    config: ServerConfig,
    runtime: RuntimeContext,
    dashboard: bool,
    instance: Option<crate::instance::Instance>,
    origins: origins::BrowserOrigins,
) -> anyhow::Result<tokio::task::JoinHandle<anyhow::Result<()>>> {
    store
        .bind_deployment(std::env::var("GAMMABOARD_DEPLOYMENT_ID").ok().as_deref())
        .await?;
    let bind = config.bind_addr();
    let state = AppState {
        instance,
        store: store.clone(),
        auth: config
            .auth
            .as_ref()
            .map(|auth| AuthConfig::from_server_config(auth, &config.name)),
        server_name: config.name.clone(),
        origins,
        secure_cookie: config.secure_cookie,
        allow_local_node_spawn: config.allow_local_node_spawn
            && !runtime.runtime_config().externally_managed,
        api_bind: bind.to_string(),
        server_config_path: config.server_config_path.clone(),
        run_templates_dir: PathBuf::from(&config.run_templates_dir),
        task_templates_dir: PathBuf::from(&config.task_templates_dir),
        node_templates_dir: PathBuf::from(&config.node_templates_dir),
        runtime: runtime.clone(),
        run_removals: run_removals::RunRemovals::default(),
        campaign_cache: campaign::CampaignCache::default(),
    };

    let mut app = routes::build_app(state);
    if dashboard
        && Path::new(&config.frontend.build_dir)
            .join("index.html")
            .is_file()
    {
        let root = PathBuf::from(&config.frontend.build_dir);
        app = app.fallback_service(
            tower_http::services::ServeDir::new(&root).not_found_service(
                tower_http::services::ServeFile::new(root.join("index.html")),
            ),
        );
    }

    println!("server listening on http://{}", bind);
    println!("api available at http://{}/api", bind);
    tracing::info!("server listening on http://{}", bind);
    tracing::info!("api available at http://{}/api", bind);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind server socket at {bind}"))?;
    Ok(tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .context("API service exited")
    }))
}

#[derive(Clone)]
pub(crate) struct AppState {
    instance: Option<crate::instance::Instance>,
    store: PgStore,
    pub(crate) auth: Option<AuthConfig>,
    server_name: String,
    origins: origins::BrowserOrigins,
    secure_cookie: bool,
    allow_local_node_spawn: bool,
    api_bind: String,
    server_config_path: PathBuf,
    run_templates_dir: PathBuf,
    task_templates_dir: PathBuf,
    node_templates_dir: PathBuf,
    runtime: RuntimeContext,
    run_removals: run_removals::RunRemovals,
    campaign_cache: campaign::CampaignCache,
}

#[derive(Deserialize)]
struct TaskPanelRequest {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    phase: TaskOutputPhase,
    #[serde(flatten)]
    request: PanelRequest,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TaskOutputPhase {
    #[default]
    Full,
    Current,
    History,
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

#[derive(Deserialize)]
struct RunsQuery {
    #[serde(default)]
    include_children: bool,
    limit: Option<usize>,
    offset: Option<usize>,
    search: Option<String>,
}

#[derive(Serialize)]
struct RunPage<T> {
    items: Vec<T>,
    next_offset: Option<usize>,
}

#[derive(Serialize)]
struct RunSummaryResponse {
    kind: String,
    archived: bool,
    run_id: i32,
    run_name: String,
    parent_run_id: Option<i32>,
    parent_run_name: Option<String>,
    spawn_label: Option<String>,
    lifecycle_state: RunLifecycleState,
    nr_completed_samples_including_children: i64,
    cpu_hours_including_children: f64,
    queue_tuning_defaults: Option<JsonValue>,
}

impl From<RunProgress> for RunSummaryResponse {
    fn from(run: RunProgress) -> Self {
        let queue_tuning_defaults = (run.kind() == "integration")
            .then_some(run.integration_params.as_ref())
            .flatten()
            .and_then(|params| {
                params
                    .pointer("/sampler_aggregator_runner_params/queue")
                    .cloned()
            });
        Self {
            kind: run.kind().to_owned(),
            archived: run.archived,
            run_id: run.run_id,
            run_name: run.run_name,
            parent_run_id: run.parent_run_id,
            parent_run_name: run.parent_run_name,
            spawn_label: run.spawn_label,
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
    #[serde(default)]
    state: gammaboard_client::protocol::WorkerFilter,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ExternalLauncher {
    launcher_id: String,
    request_id: Option<i64>,
}
#[derive(Deserialize)]
struct ExternalLease {
    lease_token: String,
    #[serde(default)]
    release: bool,
}
#[derive(Deserialize)]
struct NodeLaunchRequestProgressRequest {
    lease_token: String,
    node_name: String,
    state: String,
    job_id: Option<String>,
    error: Option<String>,
}

#[derive(Default, Deserialize)]
struct DefinitionQuery {
    #[serde(default)]
    duplicate: bool,
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
struct DefinitionResponse {
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
}

impl RunTaskResponse {
    fn new(task: RunTask) -> Self {
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
                "deployment_id": std::env::var("GAMMABOARD_DEPLOYMENT_ID").ok(),
                "build_id": crate::provenance::build_identity(),
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "database": "disconnected",
                "server_name": state.server_name,
                "deployment_id": std::env::var("GAMMABOARD_DEPLOYMENT_ID").ok(),
                "build_id": crate::provenance::build_identity(),
            })),
        )
            .into_response(),
    }
}

async fn get_session_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SessionStatus>, ApiError> {
    auth::validate_origin(&headers, &state.origins)?;
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
        .search_runs_page(
            limit,
            offset,
            params.include_children,
            params.search.as_deref().unwrap_or_default(),
        )
        .await?;
    let next_offset = (runs.len() == limit).then_some(offset + runs.len());
    json_response(RunPage {
        items: runs.into_iter().map(RunSummaryResponse::from).collect(),
        next_offset,
    })
}

async fn get_run_summary(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = state
        .store
        .get_run_progress(id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {id}")))?;
    json_response(RunSummaryResponse::from(run))
}

async fn get_workers(
    State(state): State<AppState>,
    Query(params): Query<WorkersQuery>,
) -> Result<Json<JsonValue>, ApiError> {
    json_response(
        state
            .store
            .worker_views(
                manage::worker_filter(params.state),
                params.name.as_deref(),
                params.run_id,
            )
            .await
            .map_err(crate::core::StoreError::from)?,
    )
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
    let activity = state.store.worker_activity(&worker.node_name).await?;
    let mut response = build_worker_panel_response(&worker);
    crate::server::worker_panels::append_activity_panel(&mut response, &activity);
    json_response(response)
}

async fn get_run_definition(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(query): Query<DefinitionQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut toml = run_api::export_run_definition(&state.store, id).await?;
    if query.duplicate {
        let document: toml::Value =
            toml::from_str(&toml).map_err(|err| ApiError::Internal(err.to_string()))?;
        toml = run_api::rename_run_definition(
            &toml,
            &format!("{}-copy", document["name"].as_str().unwrap_or("run")),
        )?;
    }
    json_response(DefinitionResponse { toml })
}

async fn get_task_definition(
    State(state): State<AppState>,
    AxumPath((run_id, task_id)): AxumPath<(i32, i64)>,
    Query(query): Query<DefinitionQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let toml =
        run_api::export_task_definition(&state.store, run_id, task_id, query.duplicate).await?;
    json_response(DefinitionResponse { toml })
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

    if run.kind() == "integration" {
        let checkpoint = state.store.checkpoint_status(run_id).await?;
        crate::server::run_panels::append_checkpoint_panel(&mut response, &checkpoint);
    }

    let active_task = tasks.iter().find(|task| task.state.as_str() == "active");
    let configs = match active_task {
        Some(task) if task.task.runs_on_sampler_worker() => {
            resolve_stage_context(&state.store, run_id, task, task.sequence_nr, None)
                .await
                .map(|resolved| {
                    (
                        Some((resolved.evaluator_config, resolved.evaluator_provenance)),
                        Some((
                            resolved.sampler.config().clone(),
                            resolved.sampler_provenance,
                        )),
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
    let response = tasks
        .into_iter()
        .map(RunTaskResponse::new)
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
type EffectiveSamplerConfig = (crate::core::SamplingConfig, StageConfigProvenance);

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
            .sampler_config()
            .cloned()
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
    if request.section.as_deref().is_some_and(|section| {
        !["result", "observables", "strata", "diagnostics", "summary"].contains(&section)
    }) {
        return Err(ApiError::BadRequest("Unknown task output section".into()));
    }
    let mut cursor =
        parse_task_panel_cursor(request.request.cursor.as_deref()).map_err(ApiError::BadRequest)?;
    let run = state
        .store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let task = load_run_task(&state.store, run_id, task_id).await?;
    let sample = matches!(task.task, crate::core::RunTaskSpec::Sample { .. });
    let compact = sample
        && (request.phase == TaskOutputPhase::History
            || matches!(
                request.section.as_deref(),
                Some("result" | "strata" | "summary")
            ));
    let mut latest_persisted_snapshot = if compact {
        state
            .store
            .latest_plot_snapshot(run_id, task.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
    } else {
        state
            .store
            .get_task_output_snapshots(run_id, task.id, None, 1)
            .await?
            .into_iter()
            .next()
    };
    if matches!(
        task.task,
        crate::core::RunTaskSpec::PdfAdaptationImage { .. }
            | crate::core::RunTaskSpec::PdfAdaptationPlotLine { .. }
    ) && let Some(snapshot) = &mut latest_persisted_snapshot
    {
        task_panels::pdf_adaptation::enrich_output(
            &state.store,
            &task,
            &mut snapshot.persisted_output,
            run.target.as_ref(),
            run.target_error.as_ref(),
        )
        .await?;
    }
    // Raw raster output snapshots store progress only; the stage owns the values.
    let raw_raster = matches!(
        task.task,
        RunTaskSpec::Image { .. } | RunTaskSpec::PlotLine { .. }
    );
    let latest_stage_snapshot = if latest_persisted_snapshot.is_none()
        || (raw_raster && request.phase != TaskOutputPhase::History)
    {
        state
            .store
            .get_latest_task_stage_snapshot(run_id, task.id)
            .await?
    } else {
        None
    };
    let current_accumulator = if matches!(task.state, crate::core::RunTaskState::Active)
        && request.phase != TaskOutputPhase::History
    {
        (if compact {
            state
                .store
                .current_plot_observable(run_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
        } else {
            state.store.load_current_accumulator(run_id).await?
        })
        .map(|current_accumulator| {
            AccumulatorState::from_json(&current_accumulator)
                .map_err(|err| ApiError::Internal(err.to_string()))
        })
        .transpose()?
    } else {
        None
    };
    let has_strata = current_accumulator.as_ref().is_some_and(|a| {
        matches!(a, AccumulatorState::Stratified(_))
            || a.pooled_vector().is_some_and(|v| v.partition.is_some())
    }) || latest_persisted_snapshot.as_ref().is_some_and(|s| {
        s.persisted_output
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some("stratified")
            || s.persisted_output
                .pointer("/estimate/partition")
                .is_some_and(|v| !v.is_null())
            || s.persisted_output
                .get("partition")
                .is_some_and(|v| !v.is_null())
    });
    // The current/read-model state already records its effective configuration.
    // Avoid decoding a large predecessor stage merely to recover that metadata.
    let effective_accumulator_config = if sample {
        let mut config = task
            .task
            .new_accumulator_config()
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .or_else(|| current_accumulator.as_ref().map(AccumulatorState::config));
        if config.is_none()
            && let Some(snapshot) = &latest_persisted_snapshot
        {
            let value = &snapshot.persisted_output;
            config = if let Some(saved) = value.get("config") {
                Some(
                    serde_json::from_value(saved.clone())
                        .map_err(|e| ApiError::Internal(e.to_string()))?,
                )
            } else if value.get("estimate").is_some() {
                Some(crate::core::AccumulatorConfig::Gammaloop)
            } else if value.get("components").is_some() {
                Some(
                    AccumulatorState::from_vector_persistent_json(value)
                        .map_err(|e| ApiError::Internal(e.to_string()))?
                        .config(),
                )
            } else {
                None
            };
        }
        if config.is_some() {
            config
        } else {
            crate::services::stage::try_resolve_effective_sample_accumulator_config(
                &state.store,
                run_id,
                &task,
            )
            .await?
        }
    } else {
        None
    };
    let panel_source = if matches!(
        task.task,
        crate::core::RunTaskSpec::IntegrationCampaign { .. }
    ) {
        let workers = crate::core::ControlPlaneStore::list_nodes(&state.store, None).await?;
        let publishing_core_hours = if request.phase != TaskOutputPhase::History
            && matches!(request.section.as_deref(), None | Some("result"))
        {
            state
                .store
                .campaign_publishing_core_hours(run_id, task.id)
                .await?
        } else {
            Default::default()
        };
        TaskPanelSource::for_campaign(&workers, publishing_core_hours)
    } else {
        TaskPanelSource::new(&task.task, effective_accumulator_config)
            .map_err(|err| ApiError::Internal(err.to_string()))?
    };
    let available_sections = panel_source.available_sections(has_strata);
    let panel_source = panel_source.select_section(
        request.section.as_deref(),
        request.phase == TaskOutputPhase::History,
    );
    let history = if panel_source.needs_history() && request.phase != TaskOutputPhase::Current {
        Some(
            state
                .store
                .plot_history(run_id, task.id, cursor.snapshot_id, limit)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        )
    } else {
        None
    };
    let history_reset = history.as_ref().is_some_and(|h| h.reset);
    if history_reset || request.phase == TaskOutputPhase::Current {
        cursor = Default::default();
    }
    let full_history_snapshots = history
        .as_ref()
        .filter(|h| h.reset)
        .map(|h| h.snapshots.as_slice())
        .unwrap_or(&[]);
    let delta_history_snapshots = history
        .as_ref()
        .filter(|h| !h.reset)
        .map(|h| h.snapshots.as_slice())
        .unwrap_or(&[]);
    let latest_sampler_performance = if sample && request.phase != TaskOutputPhase::History {
        state
            .store
            .get_sampler_performance_history(run_id, 1, None)
            .await?
            .into_iter()
            .next()
    } else {
        None
    };
    let live_sampler = if let Some(entry) = &latest_sampler_performance {
        state.store.get_registered_worker(&entry.worker_id).await?
    } else {
        None
    };
    let latest_sampler_performance = latest_sampler_performance.filter(|entry| {
        let age = chrono::Utc::now()
            .signed_duration_since(entry.created_at)
            .num_seconds();
        (0..=10).contains(&age)
            && live_sampler.as_ref().is_some_and(|worker| {
                worker.current_run_id == Some(run_id)
                    && worker.current_role == Some(crate::core::WorkerRole::SamplerAggregator)
                    && entry.metrics.worker.node_uuid == worker.node_uuid
                    && entry.metrics.worker.task_id.as_deref() == Some(task.id.to_string().as_str())
            })
    });
    let sampler_engine_diagnostics = latest_sampler_performance
        .as_ref()
        .map(|entry| entry.metrics.engine_diagnostics.clone());
    let (completed_samples_per_second, eta_seconds) =
        if matches!(task.task, crate::core::RunTaskSpec::Sample { .. })
            && matches!(task.state, crate::core::RunTaskState::Active)
        {
            let metrics = latest_sampler_performance
                .as_ref()
                .map(|entry| &entry.metrics);
            let completed_samples_per_second = metrics
                .as_ref()
                .map(|metrics| metrics.completed_samples_per_second)
                .filter(|value| value.is_finite() && *value > 0.0);
            let eta_seconds = metrics
                .as_ref()
                .and_then(|metrics| metrics.eta_seconds)
                .filter(|value| value.is_finite() && *value >= 0.0);
            (completed_samples_per_second, eta_seconds)
        } else {
            (None, None)
        };

    let mut payload = panel_source
        .build_response(
            format!(
                "run:{run_id}:task:{}{}",
                task.id,
                request
                    .section
                    .as_ref()
                    .map(|s| format!(":{s}"))
                    .unwrap_or_default()
            ),
            cursor,
            &task,
            &request.request.panel_state,
            run.target.as_ref(),
            run.target_error.as_ref(),
            completed_samples_per_second,
            eta_seconds,
            sampler_engine_diagnostics.as_ref(),
            current_accumulator.as_ref(),
            latest_stage_snapshot.as_ref(),
            latest_persisted_snapshot.as_ref(),
            full_history_snapshots,
            delta_history_snapshots,
        )
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    if matches!(
        task.task,
        crate::core::RunTaskSpec::IntegrationCampaign { .. }
    ) {
        for update in &mut payload.updates {
            if let panels::PanelState::Table {
                panel_id,
                payload: Some(bundle),
                ..
            } = &mut update.panel
                && panel_id == "campaign_histograms"
            {
                bundle["export_source"] = serde_json::json!({
                    "run_id": run_id, "task_id": task.id.to_string(),
                });
            }
        }
    }

    if let Some(history) = history {
        payload.cursor = Some(format!("{}:0", history.head));
    }
    payload.reset_required = history_reset;
    let mut response =
        serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?;
    if matches!(task.task, RunTaskSpec::IntegrationCampaign { .. }) {
        response["poll_after_ms"] = serde_json::json!(10000);
    }
    response["available_sections"] = serde_json::json!(available_sections);
    json_response(response)
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

async fn claim_external_node_launch_request(
    State(state): State<AppState>,
    AxumJson(payload): AxumJson<ExternalLauncher>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _admission = crate::local_control::admit(&state.store)
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let request = state
        .store
        .claim_external_launch(&payload.launcher_id, payload.request_id)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    json_response(serde_json::json!({"request":request}))
}

async fn renew_external_launch_lease(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    AxumJson(payload): AxumJson<ExternalLease>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .store
        .heartbeat_external_launch(id, &payload.lease_token, payload.release)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    json_response(serde_json::json!({"ok":true}))
}

async fn update_node_launch_request_progress(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    AxumJson(payload): AxumJson<NodeLaunchRequestProgressRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .store
        .update_external_worker(
            id,
            &payload.lease_token,
            &payload.node_name,
            &payload.state,
            payload.job_id.as_deref(),
            payload.error.as_deref(),
        )
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    json_response(serde_json::json!({"ok":true}))
}

async fn get_run_performance_graphs(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<performance_panels::history::RangeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if state.store.get_run_progress(id).await?.is_none() {
        return Err(ApiError::NotFound(format!("run {id}")));
    }
    json_response(
        performance_panels::history::load(&state.store, id, &params, false)
            .await?
            .response()?,
    )
}

async fn get_run_performance(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<performance_panels::history::RangeQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    if state.store.get_run_progress(id).await?.is_none() {
        return Err(ApiError::NotFound(format!("run {id}")));
    }
    let snapshot = crate::api::performance::snapshot(&state.store, id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let history = performance_panels::history::load(&state.store, id, &params, true).await?;
    json_response(history.performance_response(&snapshot, params.node_name.as_deref()))
}

async fn export_histogram_bundle(
    AxumJson(request): AxumJson<HistogramBundleExportRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let format = request.format.trim().to_ascii_lowercase();
    let mut wrapper = if request.payload.get("histograms").is_some() {
        request.payload
    } else {
        serde_json::json!({
            "histograms": request.payload,
        })
    };
    if let Some(object) = wrapper.as_object_mut() {
        for key in ["actions", "expands_to", "expanded_label"] {
            object.remove(key);
        }
    }
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
        "hwu" => HistogramBundleExportResponse {
            filename: "histogram_bundle.HwU".to_string(),
            mime_type: "text/plain;charset=utf-8".to_string(),
            contents: histogram_export::write_hwu(&wrapper)?,
        },
        _ => {
            return Err(ApiError::BadRequest(
                "unsupported histogram export format (expected 'json' or 'hwu')".to_string(),
            ));
        }
    };

    json_response(response)
}
/// Browser-origin configuration is an exact HTTP(S) origin, never a URL pattern.
pub fn normalize_browser_origin(origin: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(origin)?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && !origin.contains('*')
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "allowed browser origin must contain only an HTTP(S) scheme, host and optional port: {origin:?}"
    );
    Ok(parsed.origin().ascii_serialization())
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
        let value = serde_json::to_value(RunTaskResponse::new(task)).unwrap();

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
        config.api_host = "0.0.0.0".parse().unwrap();

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
    fn bundled_node_templates_parse_after_replacement_expansion() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for relative_path in [
            "resources/templates/nodes/local-two-workers.toml",
            "ops/slurm/examples/cpu-workers.toml",
            "ops/slurm/examples/free-gpu-plus-cpu.toml",
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
