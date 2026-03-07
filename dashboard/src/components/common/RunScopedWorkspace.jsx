import ConnectionStatus from "../ConnectionStatus";
import RunSelector from "../RunSelector";
import EmptyStateCard from "./EmptyStateCard";

const RunScopedWorkspace = ({
  runs,
  selectedRun,
  setSelectedRun,
  isConnected,
  noRunsMessage,
  noSelectionMessage,
  children,
}) => {
  if (!Array.isArray(runs) || runs.length === 0) {
    return (
      <>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <EmptyStateCard title="No runs available" message={noRunsMessage} />
      </>
    );
  }

  if (!selectedRun) {
    return (
      <>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
        <EmptyStateCard title="Select a run" message={noSelectionMessage} />
      </>
    );
  }

  return children;
};

export default RunScopedWorkspace;
