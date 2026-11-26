//! One PDF comparison surface for ordinary and stratified diagnostic rasters.
use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelProjector, panel_projector_with_source,
};
use crate::core::{EngineError, LineRasterGeometry, PlaneRasterGeometry};
use crate::sampling::PdfAdaptationImagePersistedOutput;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, progress_panel, sized_panel_spec,
};
use serde_json::{Value, json};

pub(super) fn projectors(geometry: PlaneRasterGeometry) -> Vec<TaskPanelProjector> {
    comparison_projectors(
        geometry.nr_points(),
        "Image Progress",
        json!({"kind":"plane","geometry":geometry}),
    )
}
pub(super) fn line_projectors(geometry: LineRasterGeometry) -> Vec<TaskPanelProjector> {
    comparison_projectors(
        geometry.nr_points(),
        "Line Progress",
        json!({"kind":"line","geometry":geometry}),
    )
}
fn output(
    ctx: &TaskPanelContext<'_>,
) -> Result<Option<PdfAdaptationImagePersistedOutput>, EngineError> {
    ctx.source
        .persisted()
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|e| EngineError::build(format!("invalid PDF diagnostic output: {e}")))
        })
        .transpose()
}
fn comparison_projectors(
    total: usize,
    label: &'static str,
    geometry: Value,
) -> Vec<TaskPanelProjector> {
    vec![
        panel_projector_with_source(
            sized_panel_spec(
                "pdf_progress",
                label,
                PanelKind::Progress,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
            TaskPanelCurrentSourcePolicy::PersistedAlways,
            move |ctx| {
                Ok(Some(progress_panel(
                    "pdf_progress",
                    output(ctx)?.map_or(ctx.task.nr_completed_samples.max(0) as f64, |o| {
                        o.processed as f64
                    }),
                    Some(total as f64),
                    Some("points"),
                    None,
                )))
            },
            |_| Ok(None),
        ),
        panel_projector_with_source(
            sized_panel_spec(
                "pdf_comparison",
                "PDF comparison",
                PanelKind::PdfComparison,
                PanelHistoryMode::None,
                PanelWidth::Full,
            ),
            TaskPanelCurrentSourcePolicy::PersistedAlways,
            move |ctx| {
                let Some(output) = output(ctx)? else {
                    return Ok(None);
                };
                if output.integrand_values.len() != total || output.pdf_values.len() != total {
                    return Err(EngineError::build("PDF diagnostic grid length mismatch"));
                }
                Ok(Some(PanelState::PdfComparison {
                    panel_id: "pdf_comparison".into(),
                    geometry: geometry.clone(),
                    data: serde_json::to_value(output)
                        .map_err(|e| EngineError::build(e.to_string()))?,
                }))
            },
            |_| Ok(None),
        ),
    ]
}

/// Enrich old outputs as well as new ones, without using statistics from a later
/// task. New rasters retain the source estimate captured when they were created.
pub(crate) async fn enrich_output<
    S: crate::core::AggregationStore + crate::core::RunTaskStore + Send + Sync,
>(
    store: &S,
    task: &crate::core::RunTask,
    data: &mut Value,
    target: Option<&Value>,
    target_error: Option<&Value>,
) -> Result<(), crate::core::StoreError> {
    use crate::sampling::pdf_reference::{configured_reference, projection, reference};
    let mut output: PdfAdaptationImagePersistedOutput = serde_json::from_value(data.clone())
        .map_err(|e| crate::core::StoreError::store(e.to_string()))?;
    let source = crate::services::stage::resolve_task_source_snapshot(
        store,
        task.run_id,
        task,
        task.task.sample_sampler_source(),
    )
    .await?;
    let source = match source {
        Some(source) => Some(source),
        None => {
            store
                .load_latest_stage_snapshot_before_sequence(task.run_id, task.sequence_nr)
                .await?
        }
    };
    if let Some(source) = &source {
        if output.projection.is_none() {
            output.projection = source.observable_state.as_ref().and_then(projection);
        }
        if output.reference.is_none() {
            output.reference = source.observable_state.as_ref().and_then(reference);
        }
        if let Some(reference) = &mut output.reference {
            reference.task_id = source.task_id;
            reference.task_name = Some(source.name.clone());
        }
    }
    if let Some(p) = &output.projection {
        if let Some(configured) = configured_reference(p, target, target_error) {
            output.reference = Some(configured);
        } else if output.reference.is_none() {
            let mut before = task.sequence_nr;
            while let Some(candidate) = store
                .load_latest_stage_snapshot_before_sequence(task.run_id, before)
                .await?
            {
                let Some(sequence) = candidate.sequence_nr.filter(|seq| *seq < before) else {
                    break;
                };
                before = sequence;
                let compatible = source.as_ref().is_some_and(|s| {
                    s.evaluator.is_some()
                        && s.evaluator == candidate.evaluator
                        && s.batch_transforms == candidate.batch_transforms
                        && serde_json::to_value(
                            s.sampler_config().map(|c| (&c.materializer, &c.transforms)),
                        )
                        .ok()
                            == serde_json::to_value(
                                candidate
                                    .sampler_config()
                                    .map(|c| (&c.materializer, &c.transforms)),
                            )
                            .ok()
                });
                if !compatible {
                    continue;
                }
                let Some(accumulator) = &candidate.observable_state else {
                    continue;
                };
                if projection(accumulator).as_ref() != Some(p) {
                    continue;
                }
                if let Some(mut found) = reference(accumulator) {
                    found.source = "previous_accumulator".into();
                    found.task_id = candidate.task_id;
                    found.task_name = Some(candidate.name);
                    output.reference = Some(found);
                    break;
                }
            }
        }
        output.target_label = Some(if p.nonnegative {
            p.label.clone()
        } else {
            format!("|{}|", p.label)
        });
    }
    if let Some(reference) = &output.reference {
        output.global_abs_integrand_norm = Some(reference.value);
    }
    *data =
        serde_json::to_value(output).map_err(|e| crate::core::StoreError::store(e.to_string()))?;
    Ok(())
}
