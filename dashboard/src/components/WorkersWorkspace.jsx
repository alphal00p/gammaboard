import { useEffect, useMemo, useState } from "react";
import { Alert, Box, FormControl, InputLabel, MenuItem, Paper, Select, Stack, Typography } from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import WorkerStatusPanel from "./WorkerStatusPanel";

const workerLabel = (worker) => {
  const role = worker?.role || "unknown";
  const node = worker?.node_id || "n/a";
  return `${worker?.worker_id || "unknown"} (${role}, node=${node})`;
};

const WorkersWorkspace = ({ workers, isConnected, lastUpdate, error }) => {
  const [selectedWorkerId, setSelectedWorkerId] = useState(null);

  useEffect(() => {
    if (workers.length === 0) {
      setSelectedWorkerId(null);
      return;
    }

    const stillExists = workers.some((worker) => worker.worker_id === selectedWorkerId);
    if (!stillExists) setSelectedWorkerId(workers[0].worker_id);
  }, [workers, selectedWorkerId]);

  const selectedWorker = useMemo(
    () => workers.find((worker) => worker.worker_id === selectedWorkerId) || null,
    [workers, selectedWorkerId],
  );
  const workerRoleCounts = useMemo(() => {
    return workers.reduce((acc, worker) => {
      const role = worker?.role || "unknown";
      acc[role] = (acc[role] || 0) + 1;
      return acc;
    }, {});
  }, [workers]);

  const activeCount = useMemo(
    () => workers.filter((worker) => (worker.status || "").toLowerCase() === "active").length,
    [workers],
  );

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />
      {error ? (
        <Alert severity="error" sx={{ mb: 2 }}>
          Failed to fetch workers.
        </Alert>
      ) : null}

      <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
        <Typography variant="h6" gutterBottom>
          Workers
        </Typography>

        {workers.length === 0 ? (
          <EmptyStateCard
            title="No workers registered"
            message="Start one or more workers to inspect assignments, heartbeat, and current role."
          />
        ) : (
          <Stack spacing={2}>
            <FormControl size="small" sx={{ maxWidth: 420 }}>
              <InputLabel id="worker-select-label">Worker</InputLabel>
              <Select
                labelId="worker-select-label"
                value={selectedWorkerId || ""}
                label="Worker"
                onChange={(event) => setSelectedWorkerId(event.target.value)}
              >
                {workers.map((worker) => (
                  <MenuItem key={worker.worker_id} value={worker.worker_id}>
                    {workerLabel(worker)}
                  </MenuItem>
                ))}
              </Select>
            </FormControl>

            <Box sx={{ display: "flex", flexWrap: "wrap", gap: 2 }}>
              <Typography variant="body2" color="text.secondary">
                total workers: <strong>{workers.length}</strong>
              </Typography>
              <Typography variant="body2" color="text.secondary">
                active: <strong>{activeCount}</strong>
              </Typography>
              {Object.entries(workerRoleCounts).map(([role, count]) => (
                <Typography key={role} variant="body2" color="text.secondary">
                  {role}: <strong>{count}</strong>
                </Typography>
              ))}
            </Box>
          </Stack>
        )}
      </Paper>

      {selectedWorker ? (
        <WorkerStatusPanel worker={selectedWorker} />
      ) : (
        <Alert severity="info">Select a worker to view assignment and heartbeat details.</Alert>
      )}
    </>
  );
};

export default WorkersWorkspace;
