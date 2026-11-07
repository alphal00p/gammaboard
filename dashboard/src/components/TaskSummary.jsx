import { Alert, Typography } from "@mui/material";
import LoadingStatus from "./common/LoadingStatus";
import PanelCollection from "./panels/PanelCollection";

const SUMMARY_PANELS = new Set(["task_summary"]);

export default function TaskSummary({ task, output }) {
  if (!task) return null;
  const { panelSpecs, panelStates, error, isLoading } = output;
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!panelSpecs.some((panel) => SUMMARY_PANELS.has(panel.panel_id))) {
    return <LoadingStatus loading={isLoading} label="Loading task summary…" />;
  }
  return <>
    <Typography variant="body2" sx={{ mb: 1 }}>Selected task: <strong>{task.name}</strong></Typography>
    <PanelCollection panelSpecs={panelSpecs.map((spec) => ({ ...spec, width: "full" }))} panelStates={panelStates} visiblePanelIds={SUMMARY_PANELS} />
  </>;
}
