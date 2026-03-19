import { Alert, FormControl, InputLabel, MenuItem, Select, Stack, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { asArray } from "../utils/collections";

const evaluatorNodeIdFor = (worker) => worker?.node_id ?? worker?.worker_id ?? null;

const PerformanceWorkspace = ({ runs, workers, selectedRun, setSelectedRun, isConnected }) => {
  const runWorkers = useMemo(
    () =>
      asArray(workers).filter(
        (worker) =>
          worker?.current_run_id === selectedRun &&
          worker?.current_role === "evaluator" &&
          evaluatorNodeIdFor(worker) != null,
      ),
    [selectedRun, workers],
  );
  const [selectedEvaluatorNodeId, setSelectedEvaluatorNodeId] = useState(null);

  useEffect(() => {
    if (runWorkers.length === 0) {
      setSelectedEvaluatorNodeId(null);
      return;
    }
    if (
      selectedEvaluatorNodeId &&
      runWorkers.some((worker) => evaluatorNodeIdFor(worker) === selectedEvaluatorNodeId)
    ) {
      return;
    }
    setSelectedEvaluatorNodeId(evaluatorNodeIdFor(runWorkers[0]));
  }, [runWorkers, selectedEvaluatorNodeId]);

  const { evaluator, sampler } = useRunPerformancePanels({
    runId: selectedRun,
    evaluatorNodeId: selectedEvaluatorNodeId,
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
          {sampler ? (
            <PanelCollection
              title="Run Throughput"
              descriptors={sampler.panels}
              currentPanels={sampler.current}
              historyItems={sampler.items}
            />
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
              value={selectedEvaluatorNodeId ?? ""}
              label="Evaluator"
              onChange={(event) => setSelectedEvaluatorNodeId(event.target.value || null)}
            >
              {runWorkers.map((worker) => {
                const nodeId = evaluatorNodeIdFor(worker);
                return (
                  <MenuItem key={nodeId} value={nodeId}>
                    {nodeId}
                  </MenuItem>
                );
              })}
            </Select>
          </FormControl>
          {selectedEvaluatorNodeId == null ? (
            <Alert severity="info">No active evaluator selected for this run.</Alert>
          ) : evaluator ? (
            <PanelCollection
              title={`Evaluator ${selectedEvaluatorNodeId}`}
              descriptors={evaluator.panels}
              currentPanels={evaluator.current}
              historyItems={evaluator.items}
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
