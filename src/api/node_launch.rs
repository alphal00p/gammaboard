//! Shared durable launch requests for CLI, dashboard, and external launchers.
use super::{ApiError, nodes as node_api, toml_template};
use crate::{core::ControlPlaneStore, runtime_context::RuntimeContext, stores::PgStore};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub struct AutoRunNodesRequest {
    pub toml: Option<String>,
    pub count: Option<usize>,
    pub max_start_failures: Option<u32>,
    #[serde(default = "empty_json_object")]
    pub args: JsonValue,
    pub name_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLaunchToml {
    groups: Vec<NodeLaunchTomlGroup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLaunchTomlGroup {
    pub count: usize,
    pub name_prefix: Option<String>,
    #[serde(default = "default_node_launch_max_start_failures")]
    pub max_start_failures: u32,
    #[serde(default = "empty_json_object")]
    pub(crate) config: JsonValue,
}

fn default_node_launch_max_start_failures() -> u32 {
    3
}

fn empty_json_object() -> JsonValue {
    JsonValue::Object(Default::default())
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedNodeLaunchGroup {
    pub count: usize,
    pub name_prefix: String,
    pub max_start_failures: u32,
    pub(crate) config: JsonValue,
    pub(crate) capabilities: BTreeMap<String, u64>,
}

mod resources;

pub(crate) fn resolve_node_launch_groups(
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
                let (config, capabilities) = resources::normalize(&group.config)?;
                Ok(ResolvedNodeLaunchGroup {
                    count: group.count,
                    name_prefix,
                    max_start_failures: group.max_start_failures,
                    config,
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

pub async fn launch(
    store: &PgStore,
    runtime: &RuntimeContext,
    local: bool,
    payload: AutoRunNodesRequest,
) -> Result<JsonValue, ApiError> {
    let groups = resolve_node_launch_groups(&payload)?;
    let groups = groups.iter().map(|g| json!({"count":g.count,"name_prefix":g.name_prefix,"max_start_failures":g.max_start_failures,"config":g.config,"capabilities":g.capabilities})).collect::<Vec<_>>();
    let mut args = payload.args;
    if !args.is_object() {
        args = json!({});
    }
    if let Some(toml) = payload.toml {
        args["toml"] = json!(toml);
    }
    let id = store
        .reserve_worker_launch_with_args(if local { "local" } else { "external" }, groups, args)
        .await
        .map_err(|e| match e {
            sqlx::Error::Protocol(message)
                if message.starts_with("worker connection budget exceeded:") =>
            {
                ApiError::BadRequest(message)
            }
            other => ApiError::Internal(other.to_string()),
        })?;
    if local {
        resolve_local_requests(store, runtime).await?;
    }
    let request = store
        .list_node_launch_requests()
        .await?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::Internal("launch request disappeared".into()))?;
    let names = request.args["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["node_names"].as_array().into_iter().flatten().cloned())
        .collect::<Vec<_>>();
    Ok(
        json!({"requested":request.requested_count,"started":request.started_count,"node_names":names,"request":request}),
    )
}

/// Claims each local request once. A failed or interrupted launch stays visible in
/// the normal queue; a second deploy must not launch a second copy.
pub async fn resolve_local_requests(
    store: &PgStore,
    runtime: &RuntimeContext,
) -> Result<(), ApiError> {
    let binary =
        crate::runtime_context::self_executable().map_err(|e| ApiError::Internal(e.to_string()))?;
    while let Some((id, args)) = store
        .claim_local_worker_launch()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        let mut workers = Vec::new();
        let mut failure = None;
        for group in args["groups"].as_array().into_iter().flatten() {
            let caps: BTreeMap<String, u64> = serde_json::from_value(group["capabilities"].clone())
                .map_err(|e| {
                    ApiError::Internal(format!("invalid canonical worker capabilities: {e}"))
                })?;
            for name in group["node_names"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
            {
                if let Err(e) = spawn_node_process(
                    &binary,
                    &runtime.runtime_cli_args(),
                    runtime,
                    name,
                    group["max_start_failures"].as_u64().unwrap_or(3) as u32,
                    &caps,
                ) {
                    failure = Some(e.to_string());
                    break;
                }
                workers.push(json!({"node_name":name}));
            }
            if failure.is_some() {
                break;
            }
        }
        let result = json!({"workers":workers});
        if let Some(error) = failure {
            node_api::mark_node_launch_request_failed(store, id, workers.len(), &result, &error)
                .await?;
            return Err(ApiError::Internal(error));
        }
        node_api::mark_node_launch_request_starting(store, id, workers.len(), &result).await?;
    }
    Ok(())
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
