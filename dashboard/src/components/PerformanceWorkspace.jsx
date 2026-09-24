import { Alert, FormControl, InputLabel, MenuItem, Select, Stack, Tab, Tabs } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import PerformanceGraphs from "./PerformanceGraphs";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import QueueTuningPanel from "./runs/QueueTuningPanel";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { useRunTasks } from "../hooks/useRunTasks";
import { updateRunTaskQueueTuning } from "../services/api";
import { asArray } from "../utils/collections";
import { compareNodesByName, nodeNameOf } from "../utils/nodes";
import { getCurrentTask } from "../utils/tasks";

const OVERVIEW_PANELS = new Set(["performance_overview", "busy_rates", "measurement_window"]);

const PerformanceWorkspace = (props) => {
  const { runs, workers, selectedRun, isConnected } = props;
  const { authenticated } = useAuth();
  const [view, setView] = useState("overview");
  const [selectedNode, setSelectedNode] = useState("");
  const [queueTuningBusy, setQueueTuningBusy] = useState(false);
  const [queueTuningMessage, setQueueTuningMessage] = useState(null);
  const [expired, setExpired] = useState(false);
  const currentRun = asArray(runs).find((entry) => entry?.run_id === selectedRun) ?? null;
  const { tasks } = useRunTasks(selectedRun, 2000);
  const sampleTask = useMemo(() => {
    const list = asArray(tasks);
    const current = getCurrentTask(list);
    return current?.is_sample ? current : list.find((task) => task?.state === "active" && task?.is_sample)
      ?? list.find((task) => task?.is_sample) ?? null;
  }, [tasks]);
  const evaluators = useMemo(() => asArray(workers)
    .filter((worker) => worker?.current_run_id === selectedRun && worker?.current_role === "evaluator")
    .sort(compareNodesByName), [workers, selectedRun]);
  const evaluatorNodeName = evaluators.some((worker) => nodeNameOf(worker) === selectedNode) ? selectedNode : null;
  const performance = useRunPerformancePanels({ runId: view === "graphs" ? null : selectedRun, evaluatorNodeName, windowSeconds: 60, pollMs: 5000 });
  const { panelSpecs, panelStates, panelValues, setPanelValue, error } = performance;

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
    } catch (err) {
      setQueueTuningMessage({ severity: "error", text: err?.message || "Failed to update queue tuning." });
    } finally {
      setQueueTuningBusy(false);
    }
  }, [sampleTask?.id, selectedRun]);

  const visible = (id) => id === "measurement_window" || (view === "overview"
    ? OVERVIEW_PANELS.has(id) : !OVERVIEW_PANELS.has(id));
  const visibleSpecs = asArray(panelSpecs).filter((spec) => visible(spec.panel_id));
  const ids = new Set(visibleSpecs.map((spec) => spec.panel_id));
  const visibleStates = asArray(panelStates).filter((state) => ids.has(state.panel_id));
  const unavailable = error || expired || !isConnected;
  // Missing values are rendered as unavailable, without changing generic panels'
  // meaning of null in configuration and result views.
  const displayStates = visibleStates.map((state) => state.kind === "key_value"
    ? { ...state, entries: asArray(state.entries).map((entry) => ({ ...entry, value: entry.value ?? "Unavailable" })) }
    : state.kind === "table"
      ? { ...state, rows: asArray(state.rows).map((row) => row.map((value) => value ?? "Unavailable")) }
      : state);

  return (
    <RunScopedWorkspace {...props}
      noRunsMessage="Create a run to inspect its measurements."
      noSelectionMessage="Pick a run to inspect its measurements.">
      <Stack spacing={2}>
        <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
          <Tabs value={view} onChange={(_, value) => setView(value)} aria-label="Performance view">
            <Tab value="overview" label="Overview" />
            <Tab value="diagnostics" label="Diagnostics" />
            <Tab value="graphs" label="Graphs" />
          </Tabs>

        </Stack>
        {view === "graphs" ? <PerformanceGraphs key={selectedRun} runId={selectedRun} /> : unavailable ? <Alert severity="warning">{error || "Measurements are unavailable until the next successful refresh."}</Alert>
          : displayStates.length ? <PanelCollection title={{ overview: "Usage", diagnostics: "Diagnostics", graphs: "Graphs" }[view]}
              panelSpecs={visibleSpecs} panelStates={displayStates}
              panelValues={panelValues} onPanelValueChange={setPanelValue} />
            : <EmptyStateCard title="Waiting for measurements" message="Current coverage and measurements will appear after the next refresh." />}
        {view === "diagnostics" && <>
          <FormControl size="small" sx={{ maxWidth: 420 }}>
            <InputLabel id="performance-evaluator-label">Evaluator detail</InputLabel>
            <Select labelId="performance-evaluator-label" label="Evaluator detail" value={evaluatorNodeName ?? ""}
              onChange={(event) => setSelectedNode(event.target.value)}>
              <MenuItem value="">All live evaluators</MenuItem>
              {evaluators.map((worker) => <MenuItem key={nodeNameOf(worker)} value={nodeNameOf(worker)}>{nodeNameOf(worker)}</MenuItem>)}
            </Select>
          </FormControl>
          {queueTuningMessage && <Alert severity={queueTuningMessage.severity}>{queueTuningMessage.text}</Alert>}
          <QueueTuningPanel key={selectedRun} run={currentRun} runId={selectedRun} task={sampleTask}
            authenticated={authenticated} busy={queueTuningBusy} onSave={saveQueueTuning}
            onClear={() => saveQueueTuning(null)} />
        </>}
      </Stack>
    </RunScopedWorkspace>
  );
};

export default PerformanceWorkspace;
