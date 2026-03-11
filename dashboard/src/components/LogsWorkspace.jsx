import { Alert, Box } from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import RunSelector from "./RunSelector";
import WorkerLogsPanel from "./WorkerLogsPanel";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useWorkerLogs } from "../hooks/useWorkerLogs";

const LogsWorkspace = ({ runs, workers, selectedRun, setSelectedRun, isConnected }) => {
  const logReader = useWorkerLogs({
    runId: selectedRun,
    workers,
    limit: 100,
  });

  return (
    <RunScopedWorkspace
      runs={runs}
      selectedRun={selectedRun}
      setSelectedRun={setSelectedRun}
      isConnected={isConnected}
      noRunsMessage="Create a run to inspect logs."
      noSelectionMessage="Pick a run to inspect its logs."
    >
      <Box>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
        <WorkerLogsPanel {...logReader} title="Run Logs" />
        {logReader.items.length === 0 && !logReader.isLoading && (
          <Alert severity="info" sx={{ mt: 2 }}>
            No logs received for this run yet.
          </Alert>
        )}
      </Box>
    </RunScopedWorkspace>
  );
};

export default LogsWorkspace;
