import { lazy, useEffect, useState } from "react";
import { Alert, Box, Typography } from "@mui/material";
import StatusChip from "./common/StatusChip";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";
import WorkspaceTabs, { TabLoadingScope, WorkspacePanel } from "./common/WorkspaceTabs";
import { TASK_OUTPUT_TABS, taskPanelSection } from "./panels/panelSections";

const CampaignDiagnostics = lazy(() => import("./runs/CampaignPanels"));
const CampaignScheduling = lazy(() => import("./runs/CampaignScheduling"));

function TaskOutputSource(props) {
  const [localView, setLocalView] = useState("result");
  const view = props.view ?? localView;
  const campaign = props.task?.task_kind === "integration_campaign";
  const custom = campaign && ["diagnostics", "scheduling"].includes(view);
  const output = useTaskOutput({ runId: props.runId, taskId: props.task?.id ?? null,
    section: custom || (!campaign && view === "scheduling") ? "result" : view, paused: custom, pollMs: campaign ? 10000 : 3000, refreshKey: props.task?.state });
  return <TaskOutputContent {...props} view={view} onViewChange={props.onViewChange ?? ((next) => setLocalView(next))} output={output} />;
}

const TaskOutputPanel = (props) => props.output
  ? <TaskOutputContent {...props} /> : <TaskOutputSource {...props} />;

const TaskOutputContent = ({ runId, task, title = "Selected task output", onSelectRun = null,
  view: controlledView, onViewChange, output }) => {
  const [localView, setLocalView] = useState("result");
  const requestedView = controlledView ?? localView;
  const changeView = onViewChange ?? setLocalView;
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, isLoading, availableSections, isRevalidating, isHistoryLoading } = output;
  const campaign = task?.task_kind === "integration_campaign";
  // Sample tasks declare stratum panel specs even when their accumulators have
  // no partition. Enable the tab once there is actual stratum output.
  const hasStrata = availableSections ? availableSections.includes("strata") : panelSpecs.some((spec) => taskPanelSection(spec) === "strata" && panelStates.some((state) => {
    if (state.panel_id !== spec.panel_id) return false;
    if (state.kind === "table") return state.rows?.length > 0;
    if (state.kind === "strata") return state.frames?.some((frame) => frame.rows?.length > 0);
    return true;
  }));
  const tabs = campaign ? [...TASK_OUTPUT_TABS, ["scheduling", "Scheduling"]] : TASK_OUTPUT_TABS;
  const disabledTabs = Object.fromEntries(tabs.filter(([key]) => key !== "result" &&
    (!(campaign && ["diagnostics", "scheduling"].includes(key)) && (availableSections ? !availableSections.includes(key) : key === "strata" && !hasStrata)))
    .map(([key]) => [key, key === "strata" ? "No stratum output for this task yet." : "Not provided by this task."]));
  const view = (!campaign && requestedView === "scheduling") || (!isLoading && disabledTabs[requestedView]) ? "result" : requestedView;
  useEffect(() => {
    if (view !== requestedView) {
      if (onViewChange) onViewChange(view, { replace: true });
      else setLocalView(view);
    }
  }, [view, requestedView, onViewChange]);

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }
  const customView = campaign && ["diagnostics", "scheduling"].includes(view);
  const progressIds = new Set(panelSpecs.filter((spec) => spec.kind === "progress").map((spec) => spec.panel_id));
  const visiblePanelIds = new Set(panelSpecs.filter((spec) => !progressIds.has(spec.panel_id) && taskPanelSection(spec) === view).map((spec) => spec.panel_id));
  const label = tabs.find(([key]) => key === view)?.[1] || "Result";
  return (
    <TabLoadingScope>
    <Box role="group" aria-label="Selected task" sx={{ display: "flex", alignItems: "center", flexWrap: "wrap", columnGap: 1, rowGap: 0.5, mb: 0.75 }}>
      <Typography variant="body2" color="text.secondary">Task</Typography>
      <Typography variant="subtitle2" sx={{ overflowWrap: "anywhere", minWidth: 0 }}>{task.name || "Unnamed task"}</Typography>
      <StatusChip status={task.state} />
      {task.task_kind && <Typography variant="caption" color="text.secondary">{task.task_kind.replaceAll("_", " ")}</Typography>}
    </Box>
    {!customView && error && <Alert severity={panelStates.length ? "warning" : "error"}>Could not refresh task output. {panelStates.length ? "Showing previously loaded data. " : ""}{error}</Alert>}
    {!customView && progressIds.size > 0 && <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} panelValues={panelValues}
      onPanelValueChange={setPanelValue} visiblePanelIds={progressIds} />}
    <WorkspaceTabs id="task-output" label="Task output" value={view} onChange={changeView} tabs={tabs} disabledTabs={disabledTabs}
      loading={!customView && (isLoading || isRevalidating || isHistoryLoading)}
      loadingLabel={isLoading ? "Loading task output…" : isRevalidating ? "Refreshing task output…" : "Loading history…"}
      level="detail" panelId="task-output-panel" />
    <Box role="tabpanel" id="task-output-panel" aria-labelledby={`task-output-tab-${view}`} aria-label={title}>
    {campaign && <>
      <WorkspacePanel id="task-output" value="scheduling" active={view === "scheduling"}><CampaignScheduling runId={runId} taskId={task.id} /></WorkspacePanel>
      <WorkspacePanel id="task-output" value="diagnostics" active={view === "diagnostics"}><CampaignDiagnostics runId={runId} taskId={task.id} /></WorkspacePanel>
    </>}
    {!customView && !isLoading && !error && !visiblePanelIds.size && <Alert severity="info">{panelSpecs.length ? `${label} is not provided by this task.` : "No task output available yet."}</Alert>}
    <PanelCollection
      panelSpecs={panelSpecs}
      panelStates={panelStates}
      panelValues={panelValues}
      onPanelValueChange={setPanelValue}
      onSelectRun={onSelectRun}
      visiblePanelIds={customView ? new Set() : visiblePanelIds}
    />
    </Box>
    </TabLoadingScope>
  );
};

export default TaskOutputPanel;
