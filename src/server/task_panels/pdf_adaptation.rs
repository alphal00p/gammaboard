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
