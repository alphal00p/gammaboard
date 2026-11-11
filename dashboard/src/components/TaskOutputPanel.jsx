import { useEffect, useState } from "react";
import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";
import WorkspaceTabs, { TabLoadingScope } from "./common/WorkspaceTabs";
import { TASK_OUTPUT_TABS, taskPanelSection } from "./panels/panelSections";

function TaskOutputSource(props) {
  const [localView, setLocalView] = useState("result");
  const view = props.view ?? localView;
  const output = useTaskOutput({ runId: props.runId, taskId: props.task?.id ?? null,
    section: view, refreshKey: props.task?.state });
  return <TaskOutputContent {...props} view={view} onViewChange={props.onViewChange ?? setLocalView} output={output} />;
}

const TaskOutputPanel = (props) => props.output
  ? <TaskOutputContent {...props} /> : <TaskOutputSource {...props} />;

const TaskOutputContent = ({ task, title = "Selected task output", onSelectRun = null,
  view: controlledView, onViewChange, output }) => {
  const [localView, setLocalView] = useState("result");
  const requestedView = controlledView ?? localView;
  const changeView = onViewChange ?? setLocalView;
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, isLoading, availableSections, isRevalidating, isHistoryLoading } = output;
  useEffect(() => {
    if (!isLoading && availableSections && !availableSections.includes("strata") && requestedView === "strata") changeView("result");
  }, [availableSections, isLoading, requestedView, changeView]);

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  // Sample tasks declare stratum panel specs even when their accumulators have
  // no partition. Only expose the tab once there is actual stratum output.
  const hasStrata = availableSections ? availableSections.includes("strata") : panelSpecs.some((spec) => taskPanelSection(spec) === "strata" && panelStates.some((state) => {
    if (state.panel_id !== spec.panel_id) return false;
    if (state.kind === "table") return state.rows?.length > 0;
    if (state.kind === "strata") return state.frames?.some((frame) => frame.rows?.length > 0);
    return true;
  }));
  const tabs = TASK_OUTPUT_TABS.filter(([key]) => key !== "strata" || hasStrata);
  const view = requestedView === "strata" && !hasStrata ? "result" : requestedView;
  const progressIds = new Set(panelSpecs.filter((spec) => spec.kind === "progress").map((spec) => spec.panel_id));
  const visiblePanelIds = new Set(panelSpecs.filter((spec) => !progressIds.has(spec.panel_id) && taskPanelSection(spec) === view).map((spec) => spec.panel_id));
  const label = TASK_OUTPUT_TABS.find(([key]) => key === view)?.[1] || "Result";
  return (
    <TabLoadingScope>
    {error && <Alert severity={panelStates.length ? "warning" : "error"}>Could not refresh task output. {panelStates.length ? "Showing previously loaded data. " : ""}{error}</Alert>}
    {progressIds.size > 0 && <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} panelValues={panelValues}
      onPanelValueChange={setPanelValue} visiblePanelIds={progressIds} />}
    <WorkspaceTabs id="task-output" label="Task output" value={view} onChange={changeView} tabs={tabs}
      loading={isLoading || isRevalidating || isHistoryLoading}
      loadingLabel={isLoading ? "Loading task output…" : isRevalidating ? "Refreshing task output…" : "Loading history…"}
      level="detail" panelId="task-output-panel" />
    <Box role="tabpanel" id="task-output-panel" aria-labelledby={`task-output-tab-${view}`} aria-label={title}>
    {!isLoading && !error && !visiblePanelIds.size && <Alert severity="info">{panelSpecs.length ? `${label} is not provided by this task.` : "No task output available yet."}</Alert>}
    <PanelCollection
      panelSpecs={panelSpecs}
      panelStates={panelStates}
      panelValues={panelValues}
      onPanelValueChange={setPanelValue}
      onSelectRun={onSelectRun}
      visiblePanelIds={visiblePanelIds}
    />
    </Box>
    </TabLoadingScope>
  );
};

export default TaskOutputPanel;
