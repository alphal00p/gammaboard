import { Alert, Box } from "@mui/material";
import WorkerLogsPanel from "./WorkerLogsPanel";
import { useWorkerLogs } from "../hooks/useWorkerLogs";

const LogsWorkspace = ({
  runs,
  workers,
  selectedRun,
}) => {
  const logReader = useWorkerLogs({
    runId: selectedRun,
    workers,
    limit: 100,
  });

  return (
    <Box>
      <WorkerLogsPanel {...logReader} runs={runs} title="Run Logs" />
      {logReader.items.length === 0 && !logReader.isLoading && (
        <Alert severity="info" sx={{ mt: 2 }}>
          No logs match the current filters.
        </Alert>
      )}
    </Box>
  );
};

export default LogsWorkspace;
