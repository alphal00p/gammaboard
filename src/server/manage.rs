//! Management operations shared by remote clients and the dashboard's domain APIs.
use super::*;
use crate::core::{ControlPlaneStore, RunReadStore, RunTaskStore};
use anyhow::{Context, Result, ensure};
use gammaboard_client::protocol::{Request as ManagementRequest, TaskAction, TemplateAction};
use serde_json::{Value, json};

pub(super) async fn handle(
    State(state): State<AppState>,
    payload: Result<AxumJson<ManagementRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let AxumJson(request) = payload.map_err(|error| ApiError::BadRequest(error.body_text()))?;
    execute(&state, request)
        .await
        .map(Json)
        .map_err(|error| ApiError::BadRequest(format!("{error:#}")))
}

async fn resolve(store: &PgStore, reference: &str) -> Result<crate::stores::RunProgress> {
    if let Ok(id) = reference.parse::<i32>()
        && let Some(run) = store.get_run_progress(id).await?
    {
        return Ok(run);
    }
    let mut runs = store.get_runs_by_name(reference).await?;
    ensure!(
        runs.len() == 1,
        "run '{reference}' matched {} runs; use an unambiguous numeric ID",
        runs.len()
    );
    Ok(runs.remove(0))
}

async fn execute(state: &AppState, request: ManagementRequest) -> Result<Value> {
    let store = &state.store;
    match request {
        ManagementRequest::Status {} => crate::local_control::status(store).await,
        ManagementRequest::Stop { wait, timeout_ms } => {
            let id = crate::local_control::request_stop(store, None, wait, timeout_ms).await?;
            Ok(json!({"stop_id":id}))
        }
        ManagementRequest::AcknowledgeCleanup { request_id } => {
            crate::local_control::acknowledge_cleanup(store, &request_id).await?;
            Ok(json!({"acknowledged":true,"request_id":request_id}))
        }
        ManagementRequest::RunCreate {
            toml,
            base_dir,
            name,
            start,
            max_evaluators,
            export_path,
        } => {
            let _admission = crate::local_control::admit(store).await?;
            let mut definition = if let Some(base) = base_dir {
                ensure!(
                    Path::new(&base).is_absolute(),
                    "base_dir must be absolute on the deployment"
                );
                run_api::parse_run_add_config_toml_from_base(&toml, Path::new(&base))?
            } else {
                run_api::parse_run_add_config_toml(&toml)?
            };
            if let Some(name) = name {
                definition.name = name;
            }
            if let Some(path) = &export_path {
                ensure!(
                    Path::new(path).is_absolute(),
                    "export_path must be absolute on the deployment"
                );
                ensure!(!Path::new(path).exists(), "output already exists: {path}");
            }
            let created = run_api::create_run(store, definition).await?;
            if let Some(path) = export_path {
                crate::local_control::register_export(store, created.run_id, Path::new(&path))
                    .await
                    .with_context(|| {
                        format!(
                            "created run_id={}; export registration failed",
                            created.run_id
                        )
                    })?;
            }
            let assignment = if start {
                Some(
                    start_run(store, created.run_id, max_evaluators)
                        .await
                        .with_context(|| {
                            format!(
                                "created run_id={}; retry run start {}",
                                created.run_id, created.run_id
                            )
                        })?,
                )
            } else {
                None
            };
            Ok(json!({"created":created,"assignment":assignment}))
        }
        ManagementRequest::RunList { name } => {
            Ok(serde_json::to_value(if let Some(name) = name {
                store.get_runs_by_name(&name).await?
            } else {
                store.get_all_runs().await?
            })?)
        }
        ManagementRequest::RunStart {
            run,
            max_evaluators,
        } => {
            let _admission = crate::local_control::admit(store).await?;
            start_run(store, resolve(store, &run).await?.run_id, max_evaluators).await
        }
        ManagementRequest::RunResults { run, task } => {
            Ok(
                crate::api::results::run_results(store, resolve(store, &run).await?.run_id, task)
                    .await?,
            )
        }
        ManagementRequest::RunInspect { run, log_limit } => {
            let run = resolve(store, &run).await?;
            let tasks = store.list_run_tasks(run.run_id).await?;
            let logs = store
                .get_runtime_logs(
                    log_limit.clamp(0, 500),
                    None,
                    Some(run.run_id),
                    true,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            Ok(
                json!({"run":run,"tasks":tasks,"logs":logs,"checkpoint":store.checkpoint_status(run.run_id).await?}),
            )
        }
        ManagementRequest::RunValidate { toml, base_dir } => {
            if let Some(base) = &base_dir {
                ensure!(
                    Path::new(base).is_absolute(),
                    "base_dir must be absolute on the deployment"
                );
            }
            let definition = if let Some(base) = base_dir {
                run_api::parse_run_add_config_toml_from_base(&toml, Path::new(&base))?
            } else {
                run_api::parse_run_add_config_toml(&toml)?
            };
            Ok(serde_json::to_value(
                tokio::task::spawn_blocking(move || run_api::validate_run(definition, false))
                    .await??,
            )?)
        }
        ManagementRequest::RunDuplicate { source, name } => {
            let _admission = crate::local_control::admit(store).await?;
            let raw = run_api::export_run_definition(store, resolve(store, &source).await?.run_id)
                .await?;
            let raw = run_api::rename_run_definition(&raw, &name)?;
            Ok(serde_json::to_value(
                run_api::create_run(store, run_api::parse_run_add_config_toml(&raw)?).await?,
            )?)
        }
        ManagementRequest::RunExport { run } => Ok(
            json!({"toml":run_api::export_run_definition(store,resolve(store,&run).await?.run_id).await?}),
        ),
        ManagementRequest::RunPause { runs, all } => {
            let mut results = Vec::new();
            for run in selected(store, runs, all).await? {
                if all && run.parent_run_id.is_some() {
                    continue;
                }
                results.push(serde_json::to_value(
                    run_api::pause_run(store, run.run_id).await?,
                )?);
            }
            Ok(json!(results))
        }
        ManagementRequest::RunRemove { runs, all, dry_run } => {
            let runs = selected(store, runs, all).await?;
            if dry_run {
                return Ok(json!({"dry_run":true,"runs":runs}));
            }
            let mut results = Vec::new();
            for run in runs {
                results.push(serde_json::to_value(
                    run_api::remove_run(store, run.run_id).await?,
                )?);
            }
            Ok(json!(results))
        }
        ManagementRequest::RunPerformance {
            run,
            duration_ms,
            interval_ms,
            max_age_ms,
            since,
            until,
            limit,
        } => {
            let id = resolve(store, &run).await?.run_id;
            if let Some(since) = since {
                let since = since.parse::<chrono::DateTime<chrono::Utc>>()?;
                let until = until
                    .map(|v| v.parse::<chrono::DateTime<chrono::Utc>>())
                    .transpose()?
                    .unwrap_or_else(chrono::Utc::now);
                return crate::api::performance::history(
                    store,
                    id,
                    since,
                    until,
                    limit.clamp(1, 10000),
                )
                .await;
            }
            if let Some(duration) = duration_ms {
                ensure!(
                    (100..=86400000).contains(&duration) && interval_ms >= 100,
                    "invalid measurement interval/duration"
                );
                return Ok(serde_json::to_value(
                    crate::api::performance::measure(
                        store,
                        id,
                        std::time::Duration::from_millis(duration),
                        std::time::Duration::from_millis(interval_ms),
                        std::time::Duration::from_millis(max_age_ms),
                    )
                    .await?,
                )?);
            }
            Ok(serde_json::to_value(
                crate::api::performance::snapshot(store, id).await?,
            )?)
        }
        ManagementRequest::RunReadiness {
            run,
            until,
            evaluators,
            max_age_ms,
        } => {
            let s = crate::api::performance::snapshot(store, resolve(store, &run).await?.run_id)
                .await?;
            ensure!(evaluators > 0, "evaluators must be positive");
            ensure!(
                until == "idle" || s.failed_tasks == 0,
                "run has failed tasks"
            );
            let ready = match until.as_str() {
                "ready" => s.ready(evaluators, std::time::Duration::from_millis(max_age_ms)),
                "idle" => s.idle(),
                "completed" => s.unfinished_tasks == 0,
                _ => anyhow::bail!("unknown wait condition"),
            };
            Ok(json!({"ready":ready,"snapshot":s}))
        }
        ManagementRequest::Task { run, action } => {
            task(store, resolve(store, &run).await?.run_id, action).await
        }
        ManagementRequest::WorkerList { name, offline } => {
            if offline {
                return Ok(json!(
                    store
                        .offline_nodes()
                        .await?
                        .into_iter()
                        .filter(|n| name.as_ref().is_none_or(|v| n["node_name"] == *v))
                        .collect::<Vec<_>>()
                ));
            }
            Ok(serde_json::to_value(
                store
                    .get_registered_worker_summaries(None)
                    .await?
                    .into_iter()
                    .filter(|n| name.as_ref().is_none_or(|name| n.node_name == *name))
                    .collect::<Vec<_>>(),
            )?)
        }
        ManagementRequest::WorkerInspect { name } => {
            let live = store
                .get_registered_worker_summaries(None)
                .await?
                .into_iter()
                .filter(|n| n.node_name == name)
                .collect::<Vec<_>>();
            let offline = store
                .offline_nodes()
                .await?
                .into_iter()
                .filter(|n| n["node_name"] == name)
                .collect::<Vec<_>>();
            ensure!(
                !live.is_empty() || !offline.is_empty(),
                "worker not found: {name}"
            );
            Ok(json!({"live":live,"offline":offline}))
        }
        ManagementRequest::WorkerLaunch { count, toml } => {
            let _admission = crate::local_control::admit(store).await?;
            let result = crate::api::node_launch::launch(
                store,
                state.allow_local_node_spawn,
                AutoRunNodesRequest {
                    toml,
                    count,
                    max_start_failures: None,
                    args: json!({}),
                    name_prefix: None,
                },
            )
            .await?;
            let id = result["request"]["id"]
                .as_str()
                .context("missing launch ID")?
                .parse()?;
            Ok(crate::api::node_launch::receipt(store, vec![id]).await?)
        }
        ManagementRequest::WorkerStart { name } => {
            let _admission = crate::local_control::admit(store).await?;
            let saved = offline(store, &name).await?;
            let id = store
                .start_offline_node(
                    &name,
                    saved["node_uuid"].as_str().context("missing identity")?,
                    state.allow_local_node_spawn,
                    false,
                )
                .await?;
            Ok(crate::api::node_launch::receipt(store, vec![id]).await?)
        }
        ManagementRequest::WorkerRestore {} => {
            let _admission = crate::local_control::admit(store).await?;
            let ids = store
                .restore_saved_workers(state.allow_local_node_spawn)
                .await?;
            Ok(crate::api::node_launch::receipt(store, ids).await?)
        }
        ManagementRequest::WorkerRequestCancel { id } => {
            store.cancel_pending_launch(id).await?;
            Ok(json!({"request_id":id.to_string(),"state":"canceled"}))
        }
        ManagementRequest::WorkerCancelRestoration { name } => {
            let saved = offline(store, &name).await?;
            Ok(
                json!({"canceled":store.cancel_worker_restoration(&name,saved["node_uuid"].as_str().context("missing worker identity")?).await?}),
            )
        }
        ManagementRequest::WorkerStop { names, all, idle } => {
            if all {
                return Ok(
                    json!({"rows_updated":node_api::stop_all_nodes(store).await?.rows_updated}),
                );
            }
            if idle {
                return Ok(
                    json!({"rows_updated":node_api::stop_idle_nodes(store).await?.rows_updated}),
                );
            }
            ensure!(!names.is_empty(), "select workers or --all/--idle");
            let mut results = Vec::new();
            for name in names {
                let r = node_api::stop_node(store, &name).await?;
                results.push(json!({"node_name":r.node_name,"rows_updated":r.rows_updated}));
            }
            Ok(json!(results))
        }
        ManagementRequest::WorkerAssign { name, run, role } => {
            let _admission = crate::local_control::admit(store).await?;
            let role = match role.as_str() {
                "evaluator" => crate::core::WorkerRole::Evaluator,
                "sampler_aggregator" => crate::core::WorkerRole::SamplerAggregator,
                _ => anyhow::bail!("invalid worker role"),
            };
            let assigned =
                node_api::assign_node(store, &name, resolve(store, &run).await?.run_id, role)
                    .await?;
            Ok(
                json!({"node_name":assigned.node_name,"run_id":assigned.run_id,"role":assigned.role.to_string()}),
            )
        }
        ManagementRequest::WorkerUnassign { name } => {
            node_api::unassign_node(store, &name).await?;
            Ok(json!({"node_name":name,"unassigned":true}))
        }
        ManagementRequest::WorkerRequests { id } => {
            let requests = node_api::list_node_launch_requests(store).await?;
            if let Some(id) = id {
                return Ok(serde_json::to_value(
                    requests
                        .into_iter()
                        .find(|r| r.id == id)
                        .context("launch request not found")?,
                )?);
            }
            Ok(serde_json::to_value(requests)?)
        }
        ManagementRequest::DatabaseStatus {} => {
            let version: Option<i32> = sqlx::query_scalar(
                "SELECT model_version FROM gammaboard_compatibility WHERE singleton",
            )
            .fetch_optional(store.pool())
            .await?;
            Ok(
                json!({"reachable":true,"managed":state.instance.as_ref().is_some_and(|i|i.managed_database),"model_version":version}),
            )
        }
        ManagementRequest::Template { kind, action } => template(state, &kind, action),
        ManagementRequest::Logs {
            run,
            worker,
            service,
            limit,
            before_id,
        } => {
            if let Some(service) = service {
                let instance = state
                    .instance
                    .as_ref()
                    .context("service logs require an instance")?;
                let name = match service.as_str() {
                    "server" | "supervisor" => "service.log",
                    "db" => "postgres.log",
                    _ => anyhow::bail!("unknown service"),
                };
                return tail(&instance.paths.log_dir.join(name), limit.min(5000));
            }
            let run = if let Some(run) = run {
                Some(resolve(store, &run).await?.run_id)
            } else {
                None
            };
            Ok(serde_json::to_value(
                store
                    .get_runtime_logs(
                        limit.min(500) as i64,
                        None,
                        run,
                        true,
                        worker.as_deref(),
                        None,
                        None,
                        None,
                        before_id,
                    )
                    .await?,
            )?)
        }
        ManagementRequest::BuildInfo {} => Ok(crate::provenance::build_info()),
    }
}

pub async fn start_run(store: &PgStore, id: i32, max_evaluators: Option<usize>) -> Result<Value> {
    if let Some(limit) = max_evaluators {
        ensure!(limit > 0, "max-evaluators must be positive");
    }
    let assigned = node_api::auto_assign_run(store, id, max_evaluators).await?;
    let workers = store.list_nodes(None).await?;
    for role in [
        crate::core::WorkerRole::SamplerAggregator,
        crate::core::WorkerRole::Evaluator,
    ] {
        ensure!(
            workers.iter().any(|n| n
                .pool_assignment
                .is_some_and(|a| a.run_id == id && a.role == role)),
            "run {id} has insufficient compatible workers for {role}; launch workers and retry run start {id}"
        );
    }
    crate::local_control::mark_started(store, id).await?;
    Ok(serde_json::to_value(assigned)?)
}

async fn selected(
    store: &PgStore,
    runs: Vec<String>,
    all: bool,
) -> Result<Vec<crate::stores::RunProgress>> {
    if all {
        return Ok(store.get_all_runs().await?);
    }
    ensure!(!runs.is_empty(), "select runs or --all");
    let mut selected = Vec::new();
    for name in runs {
        selected.push(resolve(store, &name).await?);
    }
    Ok(selected)
}
async fn offline(store: &PgStore, name: &str) -> Result<Value> {
    store
        .offline_nodes()
        .await?
        .into_iter()
        .find(|n| n["node_name"] == name)
        .context("worker is not offline")
}
async fn task(store: &PgStore, run: i32, action: TaskAction) -> Result<Value> {
    let _admission = if !matches!(
        action,
        TaskAction::List {}
            | TaskAction::Inspect { .. }
            | TaskAction::Export { .. }
            | TaskAction::Remove { .. }
    ) {
        Some(crate::local_control::admit(store).await?)
    } else {
        None
    };
    match action {
        TaskAction::List {} => Ok(serde_json::to_value(store.list_run_tasks(run).await?)?),
        TaskAction::Inspect { id } => {
            Ok(crate::api::results::run_results(store, run, Some(id)).await?)
        }
        TaskAction::Export { id } => {
            Ok(json!({"toml":run_api::export_task_definition(store,run,id,false).await?}))
        }
        TaskAction::Append { toml } => Ok(json!(
            run_api::append_tasks(store, run, run_api::parse_task_queue_toml(&toml)?)
                .await?
                .tasks
        )),
        TaskAction::Duplicate { id } => {
            let raw = run_api::export_task_definition(store, run, id, true).await?;
            Ok(json!(
                run_api::append_tasks(store, run, run_api::parse_task_queue_toml(&raw)?)
                    .await?
                    .tasks
            ))
        }
        TaskAction::Edit { id, toml } => {
            let expected = run_api::export_task_definition(store, run, id, false).await?;
            run_api::edit_pending_task(store, run, id, &toml, &expected).await?;
            Ok(json!({"run_id":run,"task_id":id}))
        }
        TaskAction::Tune { id, toml } => {
            let updated =
                run_api::update_task_queue_tuning(store, run, id, Some(toml::from_str(&toml)?))
                    .await?;
            Ok(json!({"run_id":run,"task":updated.task}))
        }
        TaskAction::Remove { id } => {
            run_api::remove_pending_task(store, run, id).await?;
            Ok(json!({"run_id":run,"task_id":id,"removed":true}))
        }
    }
}
fn template(state: &AppState, kind: &str, action: TemplateAction) -> Result<Value> {
    let dir = match kind {
        "run" => &state.run_templates_dir,
        "task" => &state.task_templates_dir,
        "worker" => &state.node_templates_dir,
        _ => anyhow::bail!("invalid template kind"),
    };
    match action {
        TemplateAction::List {} => Ok(json!(template_api::list_templates(dir)?)),
        TemplateAction::Show { name } => Ok(serde_json::to_value(template_api::load_template(
            dir, &name,
        )?)?),
        TemplateAction::Save {
            name,
            toml,
            overwrite,
        } => {
            ensure!(
                overwrite || template_api::load_template(dir, &name).is_err(),
                "template exists; use --yes to overwrite"
            );
            let _: toml::Value = toml::from_str(&toml)?;
            Ok(serde_json::to_value(template_api::save_template(
                dir, &name, &toml,
            )?)?)
        }
        TemplateAction::Remove { name } => {
            template_api::delete_template(dir, &name)?;
            Ok(json!({"removed":name}))
        }
    }
}
fn tail(path: &Path, limit: usize) -> Result<Value> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(2 * 1024 * 1024);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let raw = String::from_utf8_lossy(&bytes);
    let mut offset = start;
    let mut lines = Vec::new();
    for line in raw.split_inclusive('\n') {
        if offset != start || start == 0 {
            lines.push(json!({"id":format!("{}:{offset}",path.display()),"message":line.trim_end(),"source":"service"}));
        }
        offset += line.len() as u64;
    }
    let first = lines.len().saturating_sub(limit);
    Ok(json!({"items":&lines[first..]}))
}

pub(super) async fn backup(State(state): State<AppState>) -> Result<Response, ApiError> {
    async {
        use tokio::io::AsyncReadExt;
        let temporary = tempfile::NamedTempFile::new()?;
        let output = tokio::process::Command::new("pg_dump")
            .arg("--format=custom")
            .arg("--file")
            .arg(temporary.path())
            .arg(&state.runtime.runtime_config().database.url)
            .output()
            .await?;
        ensure!(
            output.status.success(),
            "backup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let file = tokio::fs::File::open(temporary.path()).await?;
        let stream = futures_util::stream::try_unfold(
            (file, temporary),
            |(mut file, temporary)| async move {
                let mut bytes = vec![0u8; 64 * 1024];
                let size = file.read(&mut bytes).await?;
                if size == 0 {
                    return Ok::<_, std::io::Error>(None);
                }
                bytes.truncate(size);
                Ok(Some((bytes, (file, temporary))))
            },
        );
        Ok::<_, anyhow::Error>(
            (
                [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
                axum::body::Body::from_stream(stream),
            )
                .into_response(),
        )
    }
    .await
    .map_err(|e| ApiError::Internal(format!("{e:#}")))
}
