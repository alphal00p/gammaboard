import { useCampaign } from "../hooks/useCampaign";
import { CampaignPanelResponse } from "./runs/CampaignPanels";
import CampaignQueueTuning from "./runs/CampaignQueueTuning";
import LoadingStatus from "./common/LoadingStatus";
import StableTaskOutput from "./common/StableTaskOutput";
import { Alert, FormControl, MenuItem, Select, Stack, ToggleButton, ToggleButtonGroup, Typography } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import PerformanceGraphs from "./PerformanceGraphs";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import QueueTuningPanel from "./runs/QueueTuningPanel";
import RunInfo from "./RunInfo";
import WorkspaceTabs, { TabLoadingScope, WorkspacePanel } from "./common/WorkspaceTabs";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { updateRunTaskQueueTuning } from "../services/api";
import { asArray } from "../utils/collections";
import { compareNodesByName, nodeNameOf } from "../utils/nodes";

const OVERVIEW_PANELS = new Set(["performance_overview", "busy_rates", "measurement_window"]);
const ACTIVITY_HELP = {
  title: "How activity is measured",
  text: "Time-weighted average across reporting workers in each role: 100 × sum(active seconds) / sum(observed worker seconds). Longer reporting spans carry more weight; allocated CPU cores are not part of the denominator.\n\n" +
    "Compute covers evaluation/materialization/reduction or sampler generation/training/aggregation. I/O covers queue/database operations and their waits. Waiting for new work is idle.\n\n" +
    "For expensive integrands, high evaluator compute with low fetch/submit waits usually means workers are well fed. Low compute with high fetch wait suggests insufficient input supply; high submit wait suggests publication delays or backpressure. Compare accepted samples/s and exposed waits in Diagnostics, plus live queue depth.\n\n" +
    "Unavailable means missing measurement, not zero activity. Check Measurement window for reporting coverage.",
};

const PerformanceWorkspaceContent = (props) => {
  const { runs, workers, selectedRun, isConnected, tasks = [], view: controlledView, onViewChange } = props;
  const { authenticated } = useAuth();
  const [selection, setSelection] = useState({ seconds: 30 });
  const [localView, setLocalView] = useState("overview");
  const view = controlledView ?? localView;
  const setView = onViewChange ?? setLocalView;
  const [selectedNode, setSelectedNode] = useState("");
  const [queueTuningStatus, setQueueTuningStatus] = useState(null);
  const [expired, setExpired] = useState(false);
  const currentRun = asArray(runs).find((entry) => entry?.run_id === selectedRun) ?? null;
  const campaignTask = asArray(tasks).find((task) => task.task_kind === "integration_campaign");
  const campaign = currentRun?.kind === "integration_campaign";
  const campaignData = useCampaign({ runId: campaign ? selectedRun : null, taskId: campaignTask?.id,
    section: view === "overview" ? "overview" : "settings", selection: view === "overview" ? selection : null });
  const childRuns = campaignData.data?.children?.filter((child) => child.run_id != null) ?? [];
  const [chosenChild, setChosenChild] = useState(() => window.sessionStorage.getItem(`campaign-child:${selectedRun}`));
  const scheduled = campaignData.data?.controller_output?.selected_child_run_ids ?? [];
  const childRun = childRuns.find((child) => String(child.run_id) === chosenChild)
    ?? childRuns.find((child) => scheduled.includes(child.run_id)) ?? childRuns[0];
  useEffect(() => {
    // Choose once when the list arrives; later scheduler rotations must not
    // redirect an open diagnostic view to another child.
    if (campaign && childRun && !childRuns.some((child) => String(child.run_id) === chosenChild)) {
      const id = String(childRun.run_id); setChosenChild(id); window.sessionStorage.setItem(`campaign-child:${selectedRun}`, id);
    }
  }, [campaign, childRun, childRuns, chosenChild, selectedRun]);
  const performanceRun = campaign ? childRun?.run_id : selectedRun;
  const sampleTask = asArray(tasks).find((task) => task.state === "active" && task.is_sample) ?? null;
  const tuningStatus = queueTuningStatus?.taskId === sampleTask?.id ? queueTuningStatus : null;
  const evaluators = useMemo(() => asArray(workers)
    .filter((worker) => worker?.current_run_id === performanceRun && worker?.current_role === "evaluator")
    .sort(compareNodesByName), [workers, performanceRun]);
  const evaluatorNodeName = selectedNode || null;
  const performance = useRunPerformancePanels({ runId: performanceRun, selection, pollMs: campaign ? 10000 : 5000, paused: view === "graphs" || view === "tuning" || (campaign && view === "overview") });
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, refreshing, isLoading } = performance;

  useEffect(() => {
    setExpired(false);
    if (!panelStates.length) return undefined;
    const timeout = setTimeout(() => setExpired(true), campaign ? 30000 : 10000);
    return () => clearTimeout(timeout);
  }, [panelStates, campaign]);

  const saveQueueTuning = useCallback(async (payload) => {
    if (!selectedRun || !sampleTask?.id) return false;
    const taskId = sampleTask.id;
    setQueueTuningStatus({ taskId, busy: true });
    // A save belongs to the task it started on, even if the queue advances.
    const finish = (severity, text) => setQueueTuningStatus((current) => current?.taskId === taskId
      ? { taskId, message: { severity, text } } : current);
    try {
      await updateRunTaskQueueTuning(selectedRun, taskId, payload);
      finish("success", payload == null ? "Queue tuning override cleared." : "Queue tuning updated.");
      return true;
    } catch (err) {
      finish("error", err?.message || "Failed to update queue tuning.");
      return false;
    }
  }, [sampleTask?.id, selectedRun]);

  const visible = (id) => id === "measurement_window" || (view === "overview"
    ? OVERVIEW_PANELS.has(id) : !OVERVIEW_PANELS.has(id));
  const visibleSpecs = asArray(panelSpecs).filter((spec) => visible(spec.panel_id));
  const ids = new Set(visibleSpecs.map((spec) => spec.panel_id));
  const visibleStates = asArray(panelStates).filter((state) => ids.has(state.panel_id)).map((state) =>
    state.panel_id === "evaluator_diagnostics" && evaluatorNodeName
      ? { ...state, rows: asArray(state.rows).filter((row) => row[0] === evaluatorNodeName) } : state);
  const evaluatorNames = [...new Set([
    ...evaluators.map(nodeNameOf),
    ...asArray(panelStates).filter((state) => state.panel_id === "evaluator_diagnostics").flatMap((state) => asArray(state.rows).map((row) => row[0])),
    ...(selectedNode ? [selectedNode] : []),
  ])].sort();
  const unavailable = error || (expired && !refreshing) || !isConnected;
  // Missing values are rendered as unavailable, without changing generic panels'
  // meaning of null in configuration and result views.
  const displayStates = visibleStates.map((state) => state.kind === "key_value"
    ? { ...state, entries: asArray(state.entries).map((entry) => ({ ...entry, value: entry.value ?? "Unavailable" })) }
    : state.kind === "table"
      ? { ...state, rows: asArray(state.rows).map((row) => row.map((value) => value ?? "Unavailable")),
          ...(state.panel_id === "busy_rates" && { payload: { ...state.payload,
            description: "Time-weighted averages across reporting workers. Compute and I/O can overlap; these percentages are not CPU/GPU utilization.",
            column_help: Object.fromEntries(state.columns.filter((label) => /compute|i\/o/i.test(label)).map((label) => [label, ACTIVITY_HELP.text])),
          } }),
        }
      : state);

  return (
    <TabLoadingScope>
      <Stack spacing={1}>
        <WorkspaceTabs id="performance" label="Performance view" value={view} onChange={setView} level="detail"
          tabs={[["overview", "Overview"], ["diagnostics", "Diagnostics"], ["graphs", "Graphs"], ["tuning", "Queue tuning"]]}
          disabledTabs={campaign || sampleTask ? {} : { tuning: "Queue tuning requires an active sampling task." }}
          panelId="performance-content" actions={view !== "tuning" &&
          <ToggleButtonGroup size="small" exclusive aria-label="Measurement window"
            value={selection?.seconds ?? (selection ? "custom" : "all")}
            onChange={(_, value) => { if (value != null && value !== "custom") setSelection(value === "all" ? null : { seconds: value }); }}>
            <ToggleButton value={30}>30 s</ToggleButton>
            <ToggleButton value={300}>5 min</ToggleButton>
            <ToggleButton value="all">All</ToggleButton>
            {selection && selection.seconds == null && <ToggleButton value="custom">Custom</ToggleButton>}
          </ToggleButtonGroup>
          } />
        {campaign && ["diagnostics", "graphs"].includes(view) && <Stack direction="row" spacing={1} alignItems="center">
          <Typography component="label" id="performance-child-label">Child run</Typography>
          <FormControl size="small" sx={{ minWidth: 240, maxWidth: "100%" }}>
            <Select labelId="performance-child-label" value={childRun?.run_id ?? ""} onChange={(event) => {
              const id = String(event.target.value); setChosenChild(id); setSelectedNode(""); window.sessionStorage.setItem(`campaign-child:${selectedRun}`, id);
            }}>
              {childRuns.map((child) => <MenuItem key={child.run_id} value={child.run_id}>{child.label} · {child.run_name} #{child.run_id}</MenuItem>)}
            </Select>
          </FormControl>
          {campaignData.error && <Alert severity="error">{campaignData.error.message}</Alert>}
        </Stack>}
        {view !== "tuning" && <>
        <Typography variant="body2" color="text.secondary">
          {selection?.seconds ? `Latest ${selection.seconds === 30 ? "30 seconds" : "5 minutes"} of recorded history.` : selection ? "Custom recorded interval." : "All recorded history."}
          {" "}Activity and rates share this window across tabs. Worker status, memory and report age are current.
        </Typography>
        <LoadingStatus loading={refreshing && view !== "graphs" && !(campaign && view === "overview")} label="Updating… showing the previous interval." />
        </>}
        <StableTaskOutput source={`${performanceRun}:${view}:${evaluatorNodeName ?? "all"}`}>
        <div id="performance-content" role="tabpanel" aria-labelledby={`performance-tab-${view}`}>
        {campaign && view === "overview" && <CampaignPanelResponse resource={campaignData} />}
        {view === "diagnostics" && <Stack direction="row" alignItems="center" spacing={1.5} useFlexGap flexWrap="wrap">
          <Typography component="label" id="performance-evaluator-label" htmlFor="performance-evaluator-select" variant="body2">Evaluator detail</Typography>
          <FormControl size="small" sx={{ minWidth: 220, maxWidth: "100%" }}>
            <Select id="performance-evaluator-select" labelId="performance-evaluator-label" displayEmpty value={evaluatorNodeName ?? ""}
              onChange={(event) => setSelectedNode(event.target.value)}>
              <MenuItem value="">All reporting evaluators</MenuItem>
              {evaluatorNames.map((name) => <MenuItem key={name} value={name}>{name}</MenuItem>)}
            </Select>
          </FormControl>
        </Stack>}
        {view !== "graphs" && view !== "tuning" && !(campaign && view === "overview") && <>
          <LoadingStatus loading={isLoading} label="Loading measurements…" />
          {unavailable && <Alert severity="warning">{error || "Measurements are unavailable until the next successful refresh."}{displayStates.length ? " Showing previously loaded data." : ""}</Alert>}
          {displayStates.length ? <PanelCollection
                panelSpecs={visibleSpecs} panelStates={displayStates}
                panelValues={panelValues} onPanelValueChange={setPanelValue} />
              : !isLoading && <EmptyStateCard title="Waiting for measurements" message="Current coverage and measurements will appear after the next refresh." />}
        </>}
        <WorkspacePanel id="performance" value="graphs" active={view === "graphs"}>
          <PerformanceGraphs key={performanceRun} runId={performanceRun} pollMs={campaign ? 10000 : 5000} selection={selection} onSelectionChange={setSelection} />
        </WorkspacePanel>
        <WorkspacePanel id="performance" value="tuning" active={view === "tuning"}>
          {campaign ? <CampaignQueueTuning runId={selectedRun} taskId={campaignTask?.id} resource={campaignData} /> : sampleTask ? <>
          <Typography variant="body2" sx={{ mb: 1 }}>Active task: <strong>{sampleTask.name}</strong></Typography>
          <LoadingStatus loading={Boolean(tuningStatus?.busy)} label="Saving queue tuning…" />
          {tuningStatus?.message && <Alert severity={tuningStatus.message.severity}>{tuningStatus.message.text}</Alert>}
          <QueueTuningPanel key={`${selectedRun}:${sampleTask?.id}`} run={currentRun} runId={selectedRun} task={sampleTask}
            authenticated={authenticated} busy={Boolean(tuningStatus?.busy)} onSave={saveQueueTuning}
            onClear={() => saveQueueTuning(null)} />
          <RunInfo runId={selectedRun} section="tuning" refreshKey={sampleTask.id} />
          </> : <Alert severity="info">No active sampling task. Queue tuning becomes available when sampling starts.</Alert>}
        </WorkspacePanel>
        </div>
        </StableTaskOutput>
      </Stack>
    </TabLoadingScope>
  );
};

const PerformanceWorkspace = (props) => <PerformanceWorkspaceContent key={props.selectedRun} {...props} />;

export default PerformanceWorkspace;
