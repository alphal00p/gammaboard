import { Alert, FormControl, InputLabel, MenuItem, Select, Stack, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { asArray } from "../utils/collections";

const evaluatorNodeNameFor = (worker) => worker?.node_name ?? null;

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

  const { evaluator, sampler } = useRunPerformancePanels({
    runId: selectedRun,
    evaluatorNodeName: selectedEvaluatorNodeName,
    limit: 500,
    pollMs: 5000,
  });

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
          {sampler?.sourceId ? (
            <PanelCollection title="Run Throughput" panelSpecs={sampler.panelSpecs} panelStates={sampler.panelStates} />
          ) : (
            <EmptyStateCard
              title="No run performance snapshots"
              message="Run throughput panels will appear once the sampler records snapshots."
            />
          )}
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
          {selectedEvaluatorNodeName == null ? (
            <Alert severity="info">No active evaluator selected for this run.</Alert>
          ) : evaluator?.sourceId ? (
            <PanelCollection
              title={`Evaluator ${selectedEvaluatorNodeName}`}
              panelSpecs={evaluator.panelSpecs}
              panelStates={evaluator.panelStates}
            />
          ) : (
            <EmptyStateCard
              title="No evaluator performance snapshots"
              message="Evaluator panels will appear once the selected evaluator records snapshots."
            />
          )}
          <Typography variant="body2" color="text.secondary">
            Run view shows total completed samples per second and queue remaining ratio. Evaluator view is per worker.
          </Typography>
        </Stack>
      )}
    </RunScopedWorkspace>
  );
};

export default PerformanceWorkspace;
