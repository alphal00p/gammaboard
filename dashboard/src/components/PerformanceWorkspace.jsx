import LoadingStatus from "./common/LoadingStatus";
import { Alert, FormControl, InputLabel, MenuItem, Select, Stack, ToggleButton, ToggleButtonGroup, Typography } from "@mui/material";
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
import { getCurrentTask } from "../utils/tasks";

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
  const [tuningTaskId, setTuningTaskId] = useState(null);
  const [selectedNode, setSelectedNode] = useState("");
  const [queueTuningBusy, setQueueTuningBusy] = useState(false);
  const [queueTuningMessage, setQueueTuningMessage] = useState(null);
  const [expired, setExpired] = useState(false);
  const currentRun = asArray(runs).find((entry) => entry?.run_id === selectedRun) ?? null;
  const sampleTask = useMemo(() => {
    const list = asArray(tasks);
    const current = getCurrentTask(list);
    return list.find((task) => String(task.id) === tuningTaskId && task.is_sample)
      ?? (current?.is_sample ? current : list.find((task) => task?.state === "active" && task?.is_sample))
      ?? list.find((task) => task?.is_sample && task.state === "pending") ?? list.find((task) => task?.is_sample) ?? null;
  }, [tasks, tuningTaskId]);
  useEffect(() => {
    // Once opened, keep the named task selected even if the queue advances.
    // A background transition must not redirect a draft to another task.
    if (view === "tuning" && tuningTaskId == null && sampleTask) setTuningTaskId(String(sampleTask.id));
  }, [view, sampleTask, tuningTaskId]);
  const evaluators = useMemo(() => asArray(workers)
    .filter((worker) => worker?.current_run_id === selectedRun && worker?.current_role === "evaluator")
    .sort(compareNodesByName), [workers, selectedRun]);
  const evaluatorNodeName = selectedNode || null;
  const performance = useRunPerformancePanels({ runId: selectedRun, selection, pollMs: 5000, paused: view === "graphs" || view === "tuning" });
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, refreshing, isLoading } = performance;

  useEffect(() => {
    setExpired(false);
    if (!panelStates.length) return undefined;
    const timeout = setTimeout(() => setExpired(true), 10000);
    return () => clearTimeout(timeout);
  }, [panelStates]);

  const saveQueueTuning = useCallback(async (payload) => {
    if (!selectedRun || !sampleTask?.id) return;
    setQueueTuningBusy(true);
    setQueueTuningMessage(null);
    try {
      await updateRunTaskQueueTuning(selectedRun, sampleTask.id, payload);
      setQueueTuningMessage({ severity: "success", text: payload == null ? "Queue tuning override cleared." : "Queue tuning updated." });
      return true;
    } catch (err) {
      setQueueTuningMessage({ severity: "error", text: err?.message || "Failed to update queue tuning." });
      return false;
    } finally {
      setQueueTuningBusy(false);
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
      <Stack spacing={2}>
        <WorkspaceTabs id="performance" label="Performance view" value={view} onChange={setView} level="detail"
          tabs={[["overview", "Overview"], ["diagnostics", "Diagnostics"], ["graphs", "Graphs"], ["tuning", "Queue tuning"]]}
          panelId="performance-content" />
        {view !== "tuning" && <>
        <Stack direction="row" justifyContent="center">
          <ToggleButtonGroup size="small" exclusive aria-label="Measurement window"
            value={selection?.seconds ?? (selection ? "custom" : "all")}
            onChange={(_, value) => { if (value != null && value !== "custom") setSelection(value === "all" ? null : { seconds: value }); }}>
            <ToggleButton value={30}>30 s</ToggleButton>
            <ToggleButton value={300}>5 min</ToggleButton>
            <ToggleButton value="all">All</ToggleButton>
            {selection && selection.seconds == null && <ToggleButton value="custom">Custom</ToggleButton>}
          </ToggleButtonGroup>
        </Stack>
        <Typography variant="body2" color="text.secondary">
          {selection?.seconds ? `Latest ${selection.seconds === 30 ? "30 seconds" : "5 minutes"} of recorded history.` : selection ? "Custom recorded interval." : "All recorded history."}
          {" "}Activity and rates share this window across tabs. Worker status, memory and report age are current.
        </Typography>
        <LoadingStatus loading={refreshing && view !== "graphs"} label="Updating… showing the previous interval." />
        </>}
        <div id="performance-content" role="tabpanel" aria-labelledby={`performance-tab-${view}`}>
        {view !== "graphs" && view !== "tuning" && <>
          <LoadingStatus loading={isLoading} label="Loading measurements…" />
          {unavailable ? <Alert severity="warning">{error || "Measurements are unavailable until the next successful refresh."}</Alert>
            : displayStates.length ? <PanelCollection
                panelSpecs={visibleSpecs} panelStates={displayStates}
                panelValues={panelValues} onPanelValueChange={setPanelValue} />
              : !isLoading && <EmptyStateCard title="Waiting for measurements" message="Current coverage and measurements will appear after the next refresh." />}
        </>}
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
        <WorkspacePanel id="performance" value="graphs" active={view === "graphs"}>
          <PerformanceGraphs runId={selectedRun} selection={selection} onSelectionChange={setSelection} />
        </WorkspacePanel>
        <WorkspacePanel id="performance" value="tuning" active={view === "tuning"}>
          {sampleTask && <FormControl size="small" sx={{ minWidth: 240, maxWidth: "100%", mb: 2 }}>
            <InputLabel id="queue-task-label">Sampling task</InputLabel>
            <Select labelId="queue-task-label" label="Sampling task" value={String(sampleTask.id)}
              onChange={(event) => setTuningTaskId(event.target.value)}>
              {asArray(tasks).filter((task) => task.is_sample).map((task) => <MenuItem key={task.id} value={String(task.id)}>{task.name} · {task.state}</MenuItem>)}
            </Select>
          </FormControl>}
          <LoadingStatus loading={queueTuningBusy} label="Saving queue tuning…" />
          {queueTuningMessage && <Alert severity={queueTuningMessage.severity}>{queueTuningMessage.text}</Alert>}
          <QueueTuningPanel key={`${selectedRun}:${sampleTask?.id}`} run={currentRun} runId={selectedRun} task={sampleTask}
            authenticated={authenticated} busy={queueTuningBusy} onSave={saveQueueTuning}
            onClear={() => saveQueueTuning(null)} />
          {["pending", "active"].includes(sampleTask?.state) && <RunInfo runId={selectedRun} section="tuning" refreshKey={sampleTask?.state} />}
        </WorkspacePanel>
        </div>
      </Stack>
    </TabLoadingScope>
  );
};

const PerformanceWorkspace = (props) => <PerformanceWorkspaceContent key={props.selectedRun} {...props} />;

export default PerformanceWorkspace;
