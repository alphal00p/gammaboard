import ConnectionStatus from "../ConnectionStatus";
import RunSelector from "../RunSelector";
import { asArray } from "../../utils/collections";
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
  const runList = asArray(runs);
  if (runList.length === 0) {
    return (
      <>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <EmptyStateCard title="No runs available" message={noRunsMessage} />
      </>
    );
  }

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
      <RunSelector runs={runList} selectedRun={selectedRun} onRunChange={setSelectedRun} />
      {!selectedRun ? <EmptyStateCard title="Select a run" message={noSelectionMessage} /> : children}
    </>
  );
};

export default RunScopedWorkspace;
