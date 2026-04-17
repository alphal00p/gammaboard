import { Alert, FormControl, InputLabel, MenuItem, Select, Stack } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { asArray } from "../utils/collections";

const evaluatorNodeNameFor = (worker) => worker?.node_name ?? null;
const asObject = (value) => (value && typeof value === "object" && !Array.isArray(value) ? value : {});
const isHistoryTimeseriesPanelSpec = (spec) => {
  const kind = String(spec?.kind || "");
  const history = String(spec?.history || "");
  return (kind === "scalar_timeseries" || kind === "multi_timeseries") && history !== "none";
};
const HISTORY_X_AXIS_MODE_WALL_TIME = "wall_time";
const HISTORY_X_AXIS_MODE_SAMPLER_UPTIME = "sampler_uptime";
const HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES = "completed_samples";
const HISTORY_X_AXIS_MODE_SET = new Set([
  HISTORY_X_AXIS_MODE_WALL_TIME,
  HISTORY_X_AXIS_MODE_SAMPLER_UPTIME,
  HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES,
]);
const normalizeZoomRange = (candidate) => {
  const start = Number(candidate?.start);
  const end = Number(candidate?.end);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  const normalizedStart = Math.max(0, Math.min(100, start));
  const normalizedEnd = Math.max(0, Math.min(100, end));
  if (normalizedEnd < normalizedStart) return { start: normalizedEnd, end: normalizedStart };
  return { start: normalizedStart, end: normalizedEnd };
};
const extractSharedHistoryView = (value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const zoom = normalizeZoomRange(value.zoom);
  const hasTailPinned = typeof value.tailPinned === "boolean";
  const xAxisMode = HISTORY_X_AXIS_MODE_SET.has(value.xAxisMode) ? value.xAxisMode : null;
  if (!zoom && !hasTailPinned && !xAxisMode) return null;
  const shared = {};
  if (zoom) shared.zoom = zoom;
  if (hasTailPinned) shared.tailPinned = value.tailPinned;
  if (xAxisMode) shared.xAxisMode = xAxisMode;
  return shared;
};
const mergeSharedHistoryView = (current, sharedView) => {
  const next = asObject(current) ? { ...current } : {};
  if (sharedView.zoom) next.zoom = sharedView.zoom;
  if ("tailPinned" in sharedView) next.tailPinned = sharedView.tailPinned;
  if ("xAxisMode" in sharedView && HISTORY_X_AXIS_MODE_SET.has(sharedView.xAxisMode)) {
    next.xAxisMode = sharedView.xAxisMode;
  }
  return next;
};

const PerformanceWorkspace = ({ runs, workers, selectedRun, setSelectedRun, isConnected }) => {
  const runWorkers = useMemo(
    () =>
      asArray(workers).filter(
        (worker) =>
          worker?.current_run_id === selectedRun &&
          worker?.current_role === "evaluator" &&
          evaluatorNodeNameFor(worker) != null,
      ),
    [selectedRun, workers],
  );
  const [selectedEvaluatorNodeName, setSelectedEvaluatorNodeName] = useState(null);
  const [panelValues, setPanelValues] = useState({});
  const [historyXAxisMode, setHistoryXAxisMode] = useState(HISTORY_X_AXIS_MODE_SAMPLER_UPTIME);

  useEffect(() => {
    if (runWorkers.length === 0) {
      setSelectedEvaluatorNodeName(null);
      return;
    }
    if (
      selectedEvaluatorNodeName &&
      runWorkers.some((worker) => evaluatorNodeNameFor(worker) === selectedEvaluatorNodeName)
    ) {
      return;
    }
    setSelectedEvaluatorNodeName(evaluatorNodeNameFor(runWorkers[0]));
  }, [runWorkers, selectedEvaluatorNodeName]);

  const { evaluator, runEvaluator, sampler } = useRunPerformancePanels({
    runId: selectedRun,
    evaluatorNodeName: selectedEvaluatorNodeName,
    limit: 500,
    pollMs: 5000,
  });

  useEffect(() => {
    setPanelValues({});
  }, [selectedRun, selectedEvaluatorNodeName]);

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
        .filter((spec) => isHistoryTimeseriesPanelSpec(spec))
        .map((spec) => spec?.panel_id)
        .filter((id) => typeof id === "string"),
    [evaluator?.panelSpecs, runEvaluator?.panelSpecs, sampler?.panelSpecs],
  );

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
          ...(asObject(previous?.[panelId]) ? previous[panelId] : {}),
          xAxisMode: historyXAxisMode,
        };
      });
      return merged;
    });
  }, [historyXAxisMode, sharedHistoryPanelIds]);

  const handlePanelValueChange = useCallback((panelId, nextValue) => {
    setPanelValues((previous) => {
      if (!sharedHistoryPanelIds.includes(panelId)) {
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
  }, [sharedHistoryPanelIds]);

  return (
    <RunScopedWorkspace
      runs={runs}
      selectedRun={selectedRun}
      setSelectedRun={setSelectedRun}
      isConnected={isConnected}
      noRunsMessage="Create a run to inspect persisted performance history."
      noSelectionMessage="Pick a run to inspect performance panels."
    >
      <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
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
                <MenuItem value={HISTORY_X_AXIS_MODE_SAMPLER_UPTIME}>Sampler Uptime (Default)</MenuItem>
                <MenuItem value={HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES}>Completed Samples</MenuItem>
                <MenuItem value={HISTORY_X_AXIS_MODE_WALL_TIME}>Wall Time</MenuItem>
              </Select>
            </FormControl>
            <FormControl size="small" sx={{ maxWidth: 320 }}>
              <InputLabel id="performance-evaluator-label">Evaluator</InputLabel>
              <Select
                labelId="performance-evaluator-label"
                value={selectedEvaluatorNodeName ?? ""}
                label="Evaluator"
                onChange={(event) => setSelectedEvaluatorNodeName(event.target.value || null)}
              >
                {runWorkers.map((worker) => {
                  const nodeName = evaluatorNodeNameFor(worker);
                  return (
                    <MenuItem key={nodeName} value={nodeName}>
                      {nodeName}
                    </MenuItem>
                  );
                })}
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
          {runEvaluator?.sourceId ? (
            <PanelCollection
              title="Evaluator Summary"
              panelSpecs={runEvaluator.panelSpecs}
              panelStates={runEvaluator.panelStates}
              panelValues={panelValues}
              onPanelValueChange={handlePanelValueChange}
            />
          ) : null}
          {selectedEvaluatorNodeName == null ? (
            <Alert severity="info">No active evaluator selected for this run.</Alert>
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
      )}
    </RunScopedWorkspace>
  );
};

export default PerformanceWorkspace;
