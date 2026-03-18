import { Alert } from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import PanelCollection from "./panels/PanelCollection";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";

const PerformanceWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected }) => {
  const { evaluator, sampler } = useRunPerformancePanels({
    runId: selectedRun,
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
        <>
          {evaluator ? (
            <PanelCollection
              title="Evaluator Performance"
              descriptors={evaluator.panels}
              currentPanels={evaluator.current}
              historyItems={evaluator.items}
            />
          ) : (
            <Alert severity="info" sx={{ mb: 2 }}>
              No evaluator performance snapshots available yet.
            </Alert>
          )}
          {sampler ? (
            <PanelCollection
              title="Sampler Aggregator Performance"
              descriptors={sampler.panels}
              currentPanels={sampler.current}
              historyItems={sampler.items}
            />
          ) : (
            <EmptyStateCard
              title="No sampler performance snapshots"
              message="Sampler performance panels will appear once the sampler records snapshots."
            />
          )}
        </>
      )}
    </RunScopedWorkspace>
  );
};

export default PerformanceWorkspace;
