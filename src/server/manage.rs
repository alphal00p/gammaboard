//! Management operations shared by remote clients and the dashboard's domain APIs.
use super::*;
use crate::core::{RunReadStore, RunTaskStore};
use anyhow::{Context, Result, ensure};
use gammaboard_client::protocol::{
    ExportKind, LaunchSource, Request as ManagementRequest, TaskAction, TemplateAction,
    WorkerFilter,
};
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
        ManagementRequest::Status {} => {
            let mut status=crate::local_control::status(store).await?;
            status["database"]["managed"]=json!(state.instance.as_ref().is_some_and(|instance| instance.managed_database));
            Ok(status)
        },
        ManagementRequest::Stop { wait, timeout_ms } => {
            let id = crate::local_control::request_stop(store, None, wait, timeout_ms).await?;
            Ok(json!({"stop_id":id}))
        }
        ManagementRequest::AcknowledgeCleanup { request_id } => {
            crate::local_control::acknowledge_cleanup(store, &request_id).await?;
            Ok(json!({"acknowledged":true,"request_id":request_id}))
        }
        ManagementRequest::RunCreate {
            request_id,
            duplicate,
            toml,
            base_dir,
            name,
            start,
            max_evaluators,
            export_path,
        } => {
            let _admission = crate::local_control::admit(store).await?;
            let mut payload = json!({"toml":toml,"base_dir":base_dir,"name":name,"start":start,"max_evaluators":max_evaluators,"export_path":export_path});
            // Preserve old idempotency payloads when duplication is not requested.
            if duplicate {payload["duplicate"]=true.into();}
            if let Some(key) = &request_id {
                ensure!(
                    !key.trim().is_empty() && key.len() <= 256,
                    "request_id must contain 1..256 bytes"
                );
            }
            let previous = if let Some(key) = &request_id {
                store.creation_receipt(key, &payload).await?
            } else {
                None
            };
            let created = if let Some(created) = previous {
                created
            } else {
                let mut definition = if let Some(base) = &base_dir {
                    ensure!(
                        Path::new(base).is_absolute(),
                        "base_dir must be absolute on the deployment"
                    );
                    run_api::parse_run_add_config_toml_from_base(&toml, Path::new(base))?
                } else {
                    run_api::parse_run_add_config_toml(&toml)?
                };
                if duplicate {definition=run_api::prepare_run_duplicate(store,definition).await?;}
                if let Some(name) = name {
                    definition.name = name;
                }
                if let Some(path) = &export_path {
                    ensure!(
                        Path::new(path).is_absolute(),
                        "export_path must be absolute on the deployment"
                    );
                }
                let scoped = if let Some(key) = &request_id {
                    store.with_creation_request(key.clone(), payload)
                } else {
                    store.clone()
                };
                serde_json::to_value(run_api::create_run(&scoped, definition).await?)?
            };
            let id = created["run_id"]
                .as_i64()
                .context("missing created run ID")? as i32;
            let mut receipt = json!({"request_id":request_id,"created":created,"start":{"state":"not_requested"}});
            if let Some(path) = export_path {
                receipt["export"] = match crate::local_control::register_export(
                    store,
                    id,
                    Path::new(&path),
                )
                .await
                {
                    Ok(_) => json!({"state":"registered","path":path}),
                    Err(error) => {
                        json!({"state":"failed","error":{"code":"export_registration_failed","retryable":false,"message":format!("{error:#}")}})
                    }
                };
            }
            if start {
                receipt["start"] = match start_run(store, id, max_evaluators).await {
                    Ok(result) => result["start"].clone(),
                    Err(error) => {
                        json!({"state":"failed","error":{"code":"start_failed","retryable":false,"message":format!("{error:#}")}})
                    }
                };
            }
            Ok(receipt)
        }
        ManagementRequest::RunImportResults { bundle, name } => {
            let _admission = crate::local_control::admit(store).await?;
            crate::api::results::import_results(store, bundle, name).await
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
        ManagementRequest::RunExport {run,kind} => {
            let id=resolve(store,&run).await?.run_id;
            match kind {
                ExportKind::Definition=>Ok(json!({"toml":run_api::export_run_definition(store,id).await?})),
                ExportKind::Results=>Ok(crate::api::results::run_results(store,id,None).await?),
            }
        },
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
            // Parent deletion includes descendants; never race overlapping cascades.
            let ids: std::collections::HashSet<_> = runs.iter().map(|run|run.run_id).collect();
            let mut submitted = std::collections::HashSet::new();
            let mut operations = Vec::new();
            for run in runs {
                let mut parent=run.parent_run_id;
                let mut covered=false;
                while let Some(id)=parent {
                    if ids.contains(&id) { covered=true; break; }
                    parent=store.get_run_progress(id).await?.and_then(|run|run.parent_run_id);
                }
                if covered || !submitted.insert(run.run_id) { continue; }
                let store=store.clone();
                let id=run.run_id;
                operations.push(state.run_removals.start(id,run.run_name,async move {
                    run_api::remove_run(&store,id).await.map(|_|()).map_err(|error|format!("{error:#}"))
                }).await);
            }
            Ok(json!({"operations":operations}))
        }
        ManagementRequest::RunRemovalStatus { operation_id } => {
            Ok(serde_json::to_value(state.run_removals.get(&operation_id).await.context(
                "Deletion status unavailable after restart or expiry; inspect the run list before retrying"
            )?)?)
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
            task(state, resolve(store, &run).await?.run_id, action).await
        }
        ManagementRequest::WorkerList {name,state:filter,run} => {
            let run=match run {Some(run)=>Some(resolve(store,&run).await?.run_id),None=>None};
            Ok(json!(store.worker_views(worker_filter(filter),name.as_deref(),run).await?))
        }
        ManagementRequest::WorkerInspect {name} => {
            store.worker_views(None,Some(&name),None).await?.into_iter().next().context("worker not found")
        }
        ManagementRequest::WorkerLaunch {source} => {
            let _admission=crate::local_control::admit(store).await?;
            let ids=match source {
                LaunchSource::New{count,toml,max_start_failures,name_prefix,args}=>{
                    let result=crate::api::node_launch::launch(store,state.allow_local_node_spawn,AutoRunNodesRequest{count,toml,max_start_failures,name_prefix,args:if args.is_null(){json!({})}else{args}}).await?;
                    vec![result["request"]["id"].as_str().context("missing launch ID")?.parse()?]
                },
                LaunchSource::Saved{name,expected_uuid,restore_only}=>{
                    let saved=offline(store,&name).await?;
                    let uuid=saved["node_uuid"].as_str().context("missing identity")?;
                    ensure!(expected_uuid.as_deref().is_none_or(|expected|expected==uuid),"worker identity changed; refresh the worker list");
                    vec![store.start_offline_node(&name,uuid,state.allow_local_node_spawn,restore_only).await?]
                },
                LaunchSource::RestoreSaved{}=>store.restore_saved_workers(state.allow_local_node_spawn).await?,
            };
            Ok(crate::api::node_launch::receipt(store,ids).await?)
        }
        ManagementRequest::WorkerRequestCancel { id } => {
            store.cancel_pending_launch(id).await?;
            Ok(json!({"request_id":id.to_string(),"state":"canceled"}))
        }
        ManagementRequest::WorkerCancelRestoration { name, expected_uuid } => {
            let saved = offline(store, &name).await?;
            ensure!(expected_uuid.as_deref().is_none_or(|uuid|saved["node_uuid"]==uuid),"worker identity changed; refresh the worker list");
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
        ManagementRequest::WorkerUnassign {name,all,expected_uuid}=>{
            if all {
                ensure!(name.is_none() && expected_uuid.is_none(),"choose a worker or all workers");
                return Ok(json!({"rows_updated":node_api::unassign_all_nodes(store).await?}));
            }
            let name=name.context("worker name is required")?;
            let worker=store.worker_views(None,Some(&name),None).await?.into_iter().next().context("worker not found")?;
            let uuid=worker["node_uuid"].as_str().context("missing worker identity")?;
            ensure!(expected_uuid.as_deref().is_none_or(|u|u==uuid),"worker identity changed; refresh the worker list");
            if worker["state"]=="offline" {
                ensure!(store.unassign_offline_node(&name,uuid).await?,"worker became live or has a pending launch; refresh the worker list");
            } else {node_api::unassign_node(store,&name).await?;}
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
    store.try_start_run(id, max_evaluators).await
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
        .worker_views(Some(false), Some(name), None)
        .await?
        .into_iter()
        .find(|n| n["node_name"] == name)
        .context("worker is not offline")
}
async fn task(state: &AppState, run: i32, action: TaskAction) -> Result<Value> {
    let store = &state.store;
    let _admission = if !matches!(
        action,
        TaskAction::List {} | TaskAction::Export { .. } | TaskAction::Remove { .. }
    ) {
        Some(crate::local_control::admit(store).await?)
    } else {
        None
    };
    match action {
        TaskAction::List {} => Ok(serde_json::to_value(store.list_run_tasks(run).await?)?),
        TaskAction::Export { id } => {
            Ok(json!({"toml":run_api::export_task_definition(store,run,id,false).await?}))
        }
        TaskAction::Append { toml, duplicate } => {
            let tasks = run_api::parse_task_queue_toml(&toml)?;
            Ok(json!(if duplicate {
                run_api::append_task_duplicate(store, run, tasks)
                    .await?
                    .tasks
            } else {
                run_api::append_tasks(store, run, tasks).await?.tasks
            }))
        }
        TaskAction::Duplicate { id } => {
            let raw = run_api::export_task_definition(store, run, id, true).await?;
            Ok(json!(
                run_api::append_tasks(store, run, run_api::parse_task_queue_toml(&raw)?)
                    .await?
                    .tasks
            ))
        }
        TaskAction::Edit {
            id,
            toml,
            expected_toml,
        } => {
            let expected = match expected_toml {
                Some(value) => value,
                None => run_api::export_task_definition(store, run, id, false).await?,
            };
            run_api::edit_pending_task(store, run, id, &toml, &expected).await?;
            Ok(json!({"run_id":run,"task_id":id}))
        }
        TaskAction::Tune { id, queue_tuning } => {
            let updated = run_api::update_task_queue_tuning(
                store,
                run,
                id,
                queue_tuning.map(serde_json::from_value).transpose()?,
            )
            .await?;
            Ok(json!({"run_id":run,"task":updated.task}))
        }
        TaskAction::CampaignScheduling { id, settings } => {
            Ok(campaign::scheduling(state, run, id, serde_json::from_value(settings)?).await?)
        }
        TaskAction::CampaignTuning { id, settings } => {
            Ok(campaign::tuning(state, run, id, serde_json::from_value(settings)?).await?)
        }
        TaskAction::CampaignChildrenPreview { id, edit } => {
            Ok(campaign::preview_edit(state, run, id, serde_json::from_value(edit)?).await?)
        }
        TaskAction::CampaignChildrenApply { id, edit } => {
            Ok(campaign::apply_edit(state, run, id, serde_json::from_value(edit)?).await?)
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

pub(super) fn worker_filter(filter: WorkerFilter) -> Option<bool> {
    match filter {
        WorkerFilter::Live => Some(true),
        WorkerFilter::Offline => Some(false),
        WorkerFilter::All => None,
    }
}
