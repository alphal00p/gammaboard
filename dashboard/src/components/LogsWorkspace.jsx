import { Alert, Box } from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import RunSelector from "./RunSelector";
import WorkerLogsPanel from "./WorkerLogsPanel";
import RunScopedWorkspace from "./common/RunScopedWorkspace";
import { useWorkerLogs } from "../hooks/useWorkerLogs";

const LogsWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected }) => {
  const logs = useWorkerLogs({
    runId: selectedRun,
    workerId: null,
    limit: 500,
    pollMs: 5000,
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
        <WorkerLogsPanel
          logs={logs}
          runId={selectedRun}
          runs={runs}
          title="Run Logs"
          variant="full"
          defaultLevelFilter={["error", "warn", "info"]}
        />
        {logs.length === 0 && (
          <Alert severity="info" sx={{ mt: 2 }}>
            No logs received for this run yet.
          </Alert>
        )}
      </Box>
    </RunScopedWorkspace>
  );
};

export default LogsWorkspace;
