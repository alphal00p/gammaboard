import LoadingStatus from "./common/LoadingStatus";
import { Alert, Box } from "@mui/material";
import { runPanelSection } from "./panels/panelSections";
import PanelCollection from "./panels/PanelCollection";
import { useRunPanels } from "../hooks/useRunPanels";
import DisclosureField from "./common/DisclosureField";
import { KeyValuePanel } from "./panels/BasicPanels";

const OVERVIEW_IDS = new Set(["run_identity", "run_lifecycle", "run_progress", "run_target"]);

function RunSummaryPanels({ panelSpecs, panelStates, children }) {
  const overview = panelStates.filter((panel) => OVERVIEW_IDS.has(panel.panel_id)
    && (panel.kind === "key_value" || (panel.panel_id === "run_target" && panel.kind === "text")));
  const combinedIds = new Set(overview.map((panel) => panel.panel_id));
  const entries = overview.flatMap((panel) => panel.kind === "text"
    ? [{ key: "target", label: "Target", value: panel.text }]
    : panel.entries.map((entry) => ({ ...entry, key: `${panel.panel_id}.${entry.key}` })));
  const remaining = new Set(panelSpecs.filter((panel) => runPanelSection(panel) === "summary"
    && !combinedIds.has(panel.panel_id) && panel.panel_id !== "run_checkpoint").map((panel) => panel.panel_id));
  const checkpoint = panelStates.find((panel) => panel.panel_id === "run_checkpoint");
  const checkpointStatus = checkpoint?.entries?.find((entry) => entry.key === "state")?.value;
  const checkpointError = checkpoint?.entries?.find((entry) => entry.key === "error")?.value;
  const hasCheckpointError = checkpointError && !["—", "none", "n/a"].includes(checkpointError);
  return <>
    {entries.length > 0 && <Box sx={{ mb: 3 }}><KeyValuePanel title="Run overview" state={{ panel_id: "run_overview", entries }} /></Box>}
    {children}
    {remaining.size > 0 && <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} visiblePanelIds={remaining} />}
    {checkpoint && <Box sx={{ p: 2, mb: 3, border: 1, borderColor: "divider", borderRadius: 1 }}>
      {hasCheckpointError && <Alert severity="error" sx={{ mb: 1 }}>{checkpointError}</Alert>}
      <DisclosureField label="Checkpoint recovery" value={checkpointStatus || "No checkpoint recorded"}>
        <KeyValuePanel state={{ ...checkpoint, entries: checkpoint.entries.filter((entry) => !["state", "error"].includes(entry.key)) }} />
      </DisclosureField>
    </Box>}
  </>;
}

const RunInfo = ({ runId, refreshKey, section = "summary", children }) => {
  const { panelSpecs, panelStates, error, isLoading } = useRunPanels({ runId, refreshKey });

  if (runId == null) return null;

  if (error) {
    return (
      <><Box sx={{ mb: 3 }}>
        <Alert severity="error">{error}</Alert>
      </Box>{children}</>
    );
  }

  if (!Array.isArray(panelSpecs) || panelSpecs.length === 0) {
    return <><LoadingStatus loading={isLoading} label="Loading run information…" />{children}</>;
  }

  if (section === "summary") return <RunSummaryPanels panelSpecs={panelSpecs} panelStates={panelStates}>{children}</RunSummaryPanels>;

  const orderedSpecs = section === "settings" ? [...panelSpecs].sort((left, right) => {
    const rank = (panel) => panel.panel_id.endsWith("_summary") ? 0 : panel.panel_id.endsWith("_config") ? 1 : 2;
    return rank(left) - rank(right);
  }) : panelSpecs;
  const visiblePanelIds = new Set(orderedSpecs.filter((panel) => runPanelSection(panel) === section).map((panel) => panel.panel_id));
  if (!visiblePanelIds.size) return <Alert severity="info">{section === "settings" ? "No effective evaluator or sampler configuration is available yet." : "Live queue measurements are not available yet."}</Alert>;

  return <PanelCollection panelSpecs={orderedSpecs} panelStates={panelStates} visiblePanelIds={visiblePanelIds} />;
};

export default RunInfo;
