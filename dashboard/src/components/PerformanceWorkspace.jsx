import { Alert, FormControl, InputLabel, MenuItem, Select, Stack } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import QueueTuningPanel from "./runs/QueueTuningPanel";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import {
  HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES,
  HISTORY_X_AXIS_MODE_SAMPLER_UPTIME,
  HISTORY_X_AXIS_MODE_WALL_TIME,
  extractSharedHistoryView,
  isObject,
  isSharedHistoryTimeseriesPanelSpec,
  mergeSharedHistoryView,
} from "./panels/panelView";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { useRunTasks } from "../hooks/useRunTasks";
import { updateRunTaskQueueTuning } from "../services/api";
import { asArray } from "../utils/collections";
import { compareNodesByName, nodeNameOf } from "../utils/nodes";
import { asTaskList, getCurrentTask } from "../utils/tasks";

const PerformanceWorkspace = ({
  runs,
  workers,
  selectedRun,
  setSelectedRun,
  showChildRuns,
  setShowChildRuns,
  isConnected,
  serverName,
  hasMoreRuns,
  loadMoreRuns,
  isLoadingMoreRuns,
}) => {
  const { authenticated } = useAuth();
  const [nodeRunFilter, setNodeRunFilter] = useState("selected_run");
  const [nodeActivityFilter, setNodeActivityFilter] = useState("active");
  const [nodeRoleFilter, setNodeRoleFilter] = useState("evaluator");
  const runFilterOptions = useMemo(() => {
    const options = [{ value: "selected_run", label: "Selected Run" }, { value: "all_runs", label: "All Runs" }];
    asArray(runs).forEach((run) => {
      if (!Number.isFinite(Number(run?.run_id))) return;
      options.push({
        value: `run:${Number(run.run_id)}`,
        label: typeof run?.run_name === "string" && run.run_name.trim() ? run.run_name : `Run ${run.run_id}`,
      });
    });
    options.push({ value: "unassigned", label: "Unassigned" });
    return options;
  }, [runs]);
  const filteredWorkers = useMemo(() => {
    return asArray(workers)
      .filter((worker) => nodeNameOf(worker) != null)
      .filter((worker) => {
        const runId = Number(worker?.current_run_id);
        if (nodeRunFilter === "all_runs") return true;
        if (nodeRunFilter === "selected_run") return Number.isFinite(runId) && runId === selectedRun;
        if (nodeRunFilter === "unassigned") return !Number.isFinite(runId);
        if (nodeRunFilter.startsWith("run:")) {
          const explicitRunId = Number(nodeRunFilter.slice(4));
          return Number.isFinite(runId) && Number.isFinite(explicitRunId) && runId === explicitRunId;
        }
        return true;
      })
      .filter((worker) => {
        if (nodeActivityFilter === "all") return true;
        const isActive = String(worker?.status || "").toLowerCase() === "active";
        return nodeActivityFilter === "active" ? isActive : !isActive;
      })
      .filter((worker) => {
        if (nodeRoleFilter === "all") return true;
        const role = String(worker?.current_role || "none");
        return role === nodeRoleFilter;
      })
      .sort(compareNodesByName);
  }, [nodeActivityFilter, nodeRoleFilter, nodeRunFilter, selectedRun, workers]);
  const [selectedEvaluatorNodeName, setSelectedEvaluatorNodeName] = useState(null);
  const [panelValues, setPanelValues] = useState({});
  const [historyXAxisMode, setHistoryXAxisMode] = useState(HISTORY_X_AXIS_MODE_SAMPLER_UPTIME);
  const [queueTuningBusy, setQueueTuningBusy] = useState(false);
  const [queueTuningMessage, setQueueTuningMessage] = useState(null);
  const currentRun = useMemo(
    () => asArray(runs).find((entry) => entry?.run_id === selectedRun) ?? null,
    [runs, selectedRun],
  );
  const { tasks } = useRunTasks(selectedRun, 2000);
  const sampleTask = useMemo(() => {
    const taskList = asTaskList(tasks);
    const currentTask = getCurrentTask(taskList);
    if (currentTask?.is_sample) return currentTask;
    return (
      taskList.find((task) => task?.state === "active" && task?.is_sample) ??
      taskList.find((task) => task?.is_sample) ??
      null
    );
  }, [tasks]);

  useEffect(() => {
    if (filteredWorkers.length === 0) {
      setSelectedEvaluatorNodeName(null);
      return;
    }
    if (
      selectedEvaluatorNodeName &&
      filteredWorkers.some((worker) => nodeNameOf(worker) === selectedEvaluatorNodeName)
    ) {
      return;
    }
    setSelectedEvaluatorNodeName(nodeNameOf(filteredWorkers[0]));
  }, [filteredWorkers, selectedEvaluatorNodeName]);

  const { evaluator, runEvaluator, sampler } = useRunPerformancePanels({
    runId: selectedRun,
    evaluatorNodeName: selectedEvaluatorNodeName,
    limit: 500,
    pollMs: 5000,
  });

  useEffect(() => {
    setPanelValues({});
    setQueueTuningMessage(null);
  }, [selectedRun, selectedEvaluatorNodeName]);

  const saveQueueTuning = useCallback(
    async (payload) => {
      if (!selectedRun || !sampleTask?.id) return;
      setQueueTuningBusy(true);
      setQueueTuningMessage(null);
      try {
        await updateRunTaskQueueTuning(selectedRun, sampleTask.id, payload);
        setQueueTuningMessage({ severity: "success", text: "Queue tuning updated." });
      } catch (err) {
        setQueueTuningMessage({ severity: "error", text: err?.message || "Failed to update queue tuning." });
      } finally {
        setQueueTuningBusy(false);
      }
    },
    [sampleTask?.id, selectedRun],
  );

  const clearQueueTuning = useCallback(async () => {
    if (!selectedRun || !sampleTask?.id) return;
    setQueueTuningBusy(true);
    setQueueTuningMessage(null);
    try {
      await updateRunTaskQueueTuning(selectedRun, sampleTask.id, null);
      setQueueTuningMessage({ severity: "success", text: "Queue tuning override cleared." });
    } catch (err) {
      setQueueTuningMessage({ severity: "error", text: err?.message || "Failed to clear queue tuning override." });
    } finally {
      setQueueTuningBusy(false);
    }
  }, [sampleTask?.id, selectedRun]);

  const knownPanelIds = useMemo(
    () =>
      [
        ...asArray(sampler?.panelSpecs).map((spec) => spec?.panel_id),
        ...asArray(runEvaluator?.panelSpecs).map((spec) => spec?.panel_id),
        ...asArray(evaluator?.panelSpecs).map((spec) => spec?.panel_id),
      ].filter((id) => typeof id === "string"),
    [evaluator?.panelSpecs, runEvaluator?.panelSpecs, sampler?.panelSpecs],
  );
  const sharedHistoryPanelIds = useMemo(
    () =>
      [
        ...asArray(sampler?.panelSpecs),
        ...asArray(runEvaluator?.panelSpecs),
        ...asArray(evaluator?.panelSpecs),
      ]
        .filter((spec) => isSharedHistoryTimeseriesPanelSpec(spec))
        .map((spec) => spec?.panel_id)
        .filter((id) => typeof id === "string"),
    [evaluator?.panelSpecs, runEvaluator?.panelSpecs, sampler?.panelSpecs],
  );
  const sharedHistoryPanelIdSet = useMemo(() => new Set(sharedHistoryPanelIds), [sharedHistoryPanelIds]);

  useEffect(() => {
    setPanelValues((previous) => {
      const next = {};
      let changed = false;
      for (const panelId of knownPanelIds) {
        if (panelId in previous) {
          next[panelId] = previous[panelId];
        }
      }
      if (Object.keys(next).length !== Object.keys(previous).length) changed = true;
      return changed ? next : previous;
    });
  }, [knownPanelIds]);

  useEffect(() => {
    if (sharedHistoryPanelIds.length === 0) return;
    setPanelValues((previous) => {
      const merged = { ...previous };
      sharedHistoryPanelIds.forEach((panelId) => {
        merged[panelId] = {
          ...(isObject(previous?.[panelId]) ? previous[panelId] : {}),
          xAxisMode: historyXAxisMode,
        };
      });
      return merged;
    });
  }, [historyXAxisMode, sharedHistoryPanelIds]);

  const handlePanelValueChange = useCallback((panelId, nextValue) => {
    setPanelValues((previous) => {
      if (!sharedHistoryPanelIdSet.has(panelId)) {
        return {
          ...previous,
          [panelId]: nextValue,
        };
      }
      const sharedView = extractSharedHistoryView(nextValue);
      if (!sharedView) {
        return {
          ...previous,
          [panelId]: nextValue,
        };
      }
      const targetIds = sharedHistoryPanelIds;
      if (targetIds.length <= 1) {
        return {
          ...previous,
          [panelId]: nextValue,
        };
      }
      const merged = { ...previous };
      targetIds.forEach((targetId) => {
        const sourceValue = targetId === panelId ? nextValue : previous[targetId];
        merged[targetId] = mergeSharedHistoryView(sourceValue, sharedView);
      });
      return merged;
    });
  }, [sharedHistoryPanelIdSet, sharedHistoryPanelIds]);

  return (
    <RunScopedWorkspace
      runs={runs}
      selectedRun={selectedRun}
      setSelectedRun={setSelectedRun}
      showChildRuns={showChildRuns}
      setShowChildRuns={setShowChildRuns}
      hasMoreRuns={hasMoreRuns}
      loadMoreRuns={loadMoreRuns}
      isLoadingMoreRuns={isLoadingMoreRuns}
      isConnected={isConnected}
      noRunsMessage="Create a run to inspect persisted performance history."
      noSelectionMessage="Pick a run to inspect performance panels."
    >
      <ConnectionStatus isConnected={isConnected} lastUpdate={null} serverName={serverName} />
      {selectedRun == null ? null : (
        <Stack spacing={2}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
            <FormControl size="small" sx={{ maxWidth: 320 }}>
              <InputLabel id="performance-x-axis-label">X-Axis</InputLabel>
              <Select
                labelId="performance-x-axis-label"
                value={historyXAxisMode}
                label="X-Axis"
                onChange={(event) => setHistoryXAxisMode(event.target.value)}
              >
                <MenuItem value={HISTORY_X_AXIS_MODE_SAMPLER_UPTIME}>Sampler Runner Uptime (Default)</MenuItem>
                <MenuItem value={HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES}>Completed Samples</MenuItem>
                <MenuItem value={HISTORY_X_AXIS_MODE_WALL_TIME}>Wall Time</MenuItem>
              </Select>
            </FormControl>
          </Stack>
          {sampler?.sourceId ? (
            <PanelCollection
              title="Run Throughput"
              panelSpecs={sampler.panelSpecs}
              panelStates={sampler.panelStates}
              panelValues={panelValues}
              onPanelValueChange={handlePanelValueChange}
            />
          ) : (
            <EmptyStateCard
              title="No run performance snapshots"
              message="Run throughput panels will appear once the sampler records snapshots."
            />
          )}
          <Stack spacing={1}>
            {queueTuningMessage ? (
              <Alert severity={queueTuningMessage.severity}>{queueTuningMessage.text}</Alert>
            ) : null}
            <QueueTuningPanel
              run={currentRun}
              runId={selectedRun}
              task={sampleTask}
              authenticated={authenticated}
              busy={queueTuningBusy}
              onSave={saveQueueTuning}
              onClear={clearQueueTuning}
            />
          </Stack>
          {runEvaluator?.sourceId ? (
            <PanelCollection
              title="Evaluator Summary"
              panelSpecs={runEvaluator.panelSpecs}
              panelStates={runEvaluator.panelStates}
              panelValues={panelValues}
              onPanelValueChange={handlePanelValueChange}
            />
          ) : null}
          <Stack spacing={2}>
            <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
              <FormControl size="small" sx={{ maxWidth: 260 }}>
                <InputLabel id="performance-node-run-filter-label">Run</InputLabel>
                <Select
                  labelId="performance-node-run-filter-label"
                  value={nodeRunFilter}
                  label="Run"
                  onChange={(event) => setNodeRunFilter(String(event.target.value))}
                >
                  {runFilterOptions.map((option) => (
                    <MenuItem key={option.value} value={option.value}>
                      {option.label}
                    </MenuItem>
                  ))}
                </Select>
              </FormControl>
              <FormControl size="small" sx={{ maxWidth: 220 }}>
                <InputLabel id="performance-node-status-filter-label">Status</InputLabel>
                <Select
                  labelId="performance-node-status-filter-label"
                  value={nodeActivityFilter}
                  label="Status"
                  onChange={(event) => setNodeActivityFilter(String(event.target.value))}
                >
                  <MenuItem value="active">Active</MenuItem>
                  <MenuItem value="inactive">Inactive</MenuItem>
                  <MenuItem value="all">All</MenuItem>
                </Select>
              </FormControl>
              <FormControl size="small" sx={{ maxWidth: 240 }}>
                <InputLabel id="performance-node-role-filter-label">Role</InputLabel>
                <Select
                  labelId="performance-node-role-filter-label"
                  value={nodeRoleFilter}
                  label="Role"
                  onChange={(event) => setNodeRoleFilter(String(event.target.value))}
                >
                  <MenuItem value="evaluator">Evaluator</MenuItem>
                  <MenuItem value="sampler_aggregator">Sampler</MenuItem>
                  <MenuItem value="none">None</MenuItem>
                  <MenuItem value="all">All</MenuItem>
                </Select>
              </FormControl>
            </Stack>
            <FormControl size="small" sx={{ maxWidth: 420 }}>
              <InputLabel id="performance-evaluator-label">Node</InputLabel>
              <Select
                labelId="performance-evaluator-label"
                value={selectedEvaluatorNodeName ?? ""}
                label="Node"
                onChange={(event) => setSelectedEvaluatorNodeName(event.target.value || null)}
              >
                {filteredWorkers.map((worker) => {
                  const nodeName = nodeNameOf(worker);
                  return (
                    <MenuItem key={nodeName} value={nodeName}>
                      {nodeName}
                    </MenuItem>
                  );
                })}
              </Select>
            </FormControl>
            {selectedEvaluatorNodeName == null ? (
              <Alert severity="info">No node matches the current filters.</Alert>
            ) : evaluator?.sourceId ? (
              <PanelCollection
                title={`Evaluator ${selectedEvaluatorNodeName}`}
                panelSpecs={evaluator.panelSpecs}
                panelStates={evaluator.panelStates}
                panelValues={panelValues}
                onPanelValueChange={handlePanelValueChange}
              />
            ) : (
              <EmptyStateCard
                title="No evaluator performance snapshots"
                message="Evaluator panels will appear once the selected evaluator records snapshots."
              />
            )}
          </Stack>
        </Stack>
      )}
    </RunScopedWorkspace>
  );
};

export default PerformanceWorkspace;
