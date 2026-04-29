#![allow(dead_code)]

use crate::core::{
    EvaluatorConfig, RunExposedArtifact, RunExposedInfoCache, RunExposedInfoContent,
    RunExposedInfoEntry, RunExposedInfoScope, RunExposedInfoStatus, RunSpec,
};
use crate::evaluation::GammaLoopParams;
use crate::evaluation::evaluator::gammaloop::GammaLoopEvaluator;
use crate::stores::PgStore;
use crate::{BuildError, api::ApiError, resources::resolve_resource_path};
use chrono::Utc;
use gammaloop_api::state::State;
use gammalooprs::processes::DotExportSettings;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const GAMMALOOP_PROCESS_VIS_KEY: &str = "gammaloop.process_visualization";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GammaloopProcessVisualizationCacheContext {
    run_id: i32,
    params: GammaLoopParams,
    schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GammaloopProcessVisualizationMetadata {
    run_id: i32,
    state_folder: String,
    process_id: String,
    integrand_name: String,
    dot_file: String,
}

pub(crate) async fn ensure_run_exposed_info(
    store: &PgStore,
    run_id: i32,
    run_spec: &RunSpec,
) -> Result<RunExposedInfoCache, ApiError> {
    let mut cache = store.load_run_exposed_info(run_id).await?;
    let EvaluatorConfig::Gammaloop { params } = &run_spec.evaluator else {
        return Ok(cache);
    };

    let context = GammaloopProcessVisualizationCacheContext {
        run_id,
        params: params.clone(),
        schema_version: 1,
    };
    let cache_key = serde_json::to_string(&context).map_err(|err| {
        ApiError::Internal(format!(
            "failed to serialize gammaloop visualization cache context: {err}"
        ))
    })?;
    let cache_hit = cache
        .entries
        .get(GAMMALOOP_PROCESS_VIS_KEY)
        .map(|entry| entry.cache_key == cache_key)
        .unwrap_or(false);
    if cache_hit {
        return Ok(cache);
    }

    let params = params.clone();
    let entry = tokio::task::spawn_blocking(move || {
        build_gammaloop_process_visualization_entry(run_id, params, cache_key)
    })
    .await
    .map_err(|err| ApiError::Internal(format!("process visualization task join failure: {err}")))?;
    cache
        .entries
        .insert(GAMMALOOP_PROCESS_VIS_KEY.to_string(), entry);
    store.save_run_exposed_info(run_id, &cache).await?;
    Ok(cache)
}

fn build_gammaloop_process_visualization_entry(
    run_id: i32,
    params: GammaLoopParams,
    cache_key: String,
) -> RunExposedInfoEntry {
    let now = Utc::now();
    match generate_process_visualization(run_id, &params) {
        Ok(rendered) => {
            let metadata = serde_json::to_value(&rendered.metadata).ok();
            RunExposedInfoEntry {
                kind: GAMMALOOP_PROCESS_VIS_KEY.to_string(),
                title: "Process Visualization".to_string(),
                status: RunExposedInfoStatus::Ready,
                cache_key,
                scope: RunExposedInfoScope::default(),
                content: Some(RunExposedInfoContent::ArtifactBundle {
                    primary: RunExposedArtifact::Svg { data: rendered.svg },
                    attachments: vec![
                        crate::core::NamedRunExposedArtifact {
                            name: "graph.dot".to_string(),
                            artifact: RunExposedArtifact::Text {
                                mime: "text/vnd.graphviz".to_string(),
                                data: rendered.dot,
                            },
                        },
                        crate::core::NamedRunExposedArtifact {
                            name: "edge-style.typ".to_string(),
                            artifact: RunExposedArtifact::Text {
                                mime: "text/plain".to_string(),
                                data: rendered.edge_style_typst,
                            },
                        },
                    ],
                }),
                metadata,
                error: None,
                updated_at: now,
            }
        }
        Err(err) => RunExposedInfoEntry {
            kind: GAMMALOOP_PROCESS_VIS_KEY.to_string(),
            title: "Process Visualization".to_string(),
            status: RunExposedInfoStatus::Error,
            cache_key,
            scope: RunExposedInfoScope::default(),
            content: None,
            metadata: serde_json::to_value(&GammaloopProcessVisualizationMetadata {
                run_id,
                state_folder: params.state_folder.display().to_string(),
                process_id: "<unresolved>".to_string(),
                integrand_name: "<unresolved>".to_string(),
                dot_file: "<unknown>".to_string(),
            })
            .ok(),
            error: Some(err),
            updated_at: now,
        },
    }
}

struct RenderedProcessVisualization {
    svg: String,
    dot: String,
    edge_style_typst: String,
    metadata: GammaloopProcessVisualizationMetadata,
}

fn generate_process_visualization(
    run_id: i32,
    params: &GammaLoopParams,
) -> Result<RenderedProcessVisualization, String> {
    let temp_dir =
        tempfile::tempdir().map_err(|err| format!("failed to create temp dir: {err}"))?;
    let resolved_state_folder = resolve_resource_path(&params.state_folder).map_err(|err| {
        format!(
            "failed to resolve gammaloop state_folder '{}': {err}",
            params.state_folder.display()
        )
    })?;
    let mut state = State::load(resolved_state_folder.clone(), None, None).map_err(|err| {
        format!(
            "failed to load state from {}: {err}",
            resolved_state_folder.display()
        )
    })?;
    GammaLoopEvaluator::run_post_load_commands(params, &mut state)
        .map_err(build_error_to_string)?;
    let (process_id, integrand_name) = state
        .find_integrand_ref(params.process_id.as_ref(), params.integrand_name.as_ref())
        .map_err(|err| format!("failed to resolve integrand for visualization: {err}"))?;

    let export_root = temp_dir.path().join("export");
    state
        .process_list
        .export_dot(&export_root, &DotExportSettings::default())
        .map_err(|err| format!("failed to export process DOT graphs: {err}"))?;
    let edge_style_path = export_root.join("edge-style.typ");
    state
        .model
        .generate_edge_style_template(&edge_style_path)
        .map_err(|err| format!("failed to generate edge-style.typ: {err}"))?;

    let dot_files = collect_files_with_extension(&export_root, OsStr::new("dot"))
        .map_err(|err| format!("failed to collect exported DOT files: {err}"))?;
    let dot_path = dot_files
        .first()
        .cloned()
        .ok_or_else(|| "no DOT files were exported".to_string())?;
    let dot_text = fs::read_to_string(&dot_path)
        .map_err(|err| format!("failed to read DOT file {}: {err}", dot_path.display()))?;
    let edge_style_text = fs::read_to_string(&edge_style_path).map_err(|err| {
        format!(
            "failed to read edge-style template {}: {err}",
            edge_style_path.display()
        )
    })?;

    let build_dir = temp_dir.path().join("linnet-build");
    run_command(
        Command::new("linnet")
            .arg("--build-dir")
            .arg(&build_dir)
            .arg("draw")
            .arg(&export_root),
        "running linnet",
    )?;
    let pdf_files = collect_files_with_extension(&build_dir.join("figs"), OsStr::new("pdf"))
        .map_err(|err| format!("failed to collect rendered PDFs: {err}"))?;
    let pdf_path = pdf_files
        .first()
        .cloned()
        .ok_or_else(|| "linnet rendered no figure PDFs".to_string())?;

    let svg_base = temp_dir.path().join("figure_svg");
    run_command(
        Command::new("pdftocairo")
            .arg("-svg")
            .arg(&pdf_path)
            .arg(&svg_base),
        "converting rendered PDF to SVG",
    )?;
    let svg_text = fs::read_to_string(&svg_base)
        .map_err(|err| format!("failed to read SVG output {}: {err}", svg_base.display()))?;

    Ok(RenderedProcessVisualization {
        svg: svg_text,
        dot: dot_text,
        edge_style_typst: edge_style_text,
        metadata: GammaloopProcessVisualizationMetadata {
            run_id,
            state_folder: params.state_folder.display().to_string(),
            process_id: format!("{process_id:?}"),
            integrand_name,
            dot_file: dot_path.display().to_string(),
        },
    })
}

fn collect_files_with_extension(
    root: &Path,
    extension: &OsStr,
) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_files_recursive(root, extension, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(
    root: &Path,
    extension: &OsStr,
    out: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, extension, out)?;
            continue;
        }
        if path.extension() == Some(extension) {
            out.push(path);
        }
    }
    Ok(())
}

fn run_command(command: &mut Command, description: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|err| format!("failed while {description}: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "{description} failed (status={}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout.trim(),
        stderr.trim()
    ))
}

fn build_error_to_string(err: BuildError) -> String {
    err.to_string()
}
