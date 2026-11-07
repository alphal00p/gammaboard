import LoadingStatus from "./common/LoadingStatus";
import { useState } from "react";
import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";
import WorkspaceTabs from "./common/WorkspaceTabs";
import { TASK_OUTPUT_TABS, taskPanelSection } from "./panels/panelSections";

function TaskOutputSource(props) {
  const output = useTaskOutput({ runId: props.runId, taskId: props.task?.id ?? null,
    pollMs: 2000, refreshKey: props.task?.state, panelLimit: 500 });
  return <TaskOutputContent {...props} output={output} />;
}

const TaskOutputPanel = (props) => props.output
  ? <TaskOutputContent {...props} /> : <TaskOutputSource {...props} />;

const TaskOutputContent = ({ task, title = "Selected task output", onSelectRun = null,
  view: controlledView, onViewChange, output }) => {
  const [localView, setLocalView] = useState("result");
  const requestedView = controlledView ?? localView;
  const changeView = onViewChange ?? setLocalView;
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, isLoading } = output;

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  if (error) {
    return <Alert severity="error">{error}</Alert>;
  }

  if (panelSpecs.length === 0) return isLoading
    ? <LoadingStatus loading label="Loading task output…" />
    : <Alert severity="info">No task output available yet.</Alert>;

  // Sample tasks declare stratum panel specs even when their accumulators have
  // no partition. Only expose the tab once there is actual stratum output.
  const hasStrata = panelSpecs.some((spec) => taskPanelSection(spec) === "strata" && panelStates.some((state) => {
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
    <>
    {progressIds.size > 0 && <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} panelValues={panelValues}
      onPanelValueChange={setPanelValue} visiblePanelIds={progressIds} />}
    <WorkspaceTabs id="task-output" label="Task output" value={view} onChange={changeView} tabs={tabs}
      level="detail" panelId="task-output-panel" />
    <Box role="tabpanel" id="task-output-panel" aria-labelledby={`task-output-tab-${view}`} aria-label={title}>
    {!visiblePanelIds.size && <Alert severity="info">{label} is not provided by this task.</Alert>}
    <PanelCollection
      panelSpecs={panelSpecs}
      panelStates={panelStates}
      panelValues={panelValues}
      onPanelValueChange={setPanelValue}
      onSelectRun={onSelectRun}
      visiblePanelIds={visiblePanelIds}
    />
    </Box>
    </>
  );
};

export default TaskOutputPanel;
