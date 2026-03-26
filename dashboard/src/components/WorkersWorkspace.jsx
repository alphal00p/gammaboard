import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import WorkerDetailsPanel from "./WorkerDetailsPanel";
import EmptyStateCard from "./common/EmptyStateCard";
import { formatDateTime } from "../utils/formatters";

const WorkersWorkspace = ({ workers, runs, isConnected, lastUpdate, error }) => {
  const [selectedNodeName, setSelectedNodeName] = useState(null);
  const nodeNameFor = (worker) => worker.node_name || null;
  const sortedWorkers = useMemo(
    () =>
      [...workers].sort((left, right) =>
        String(nodeNameFor(left) || "").localeCompare(String(nodeNameFor(right) || ""), undefined, {
          numeric: true,
          sensitivity: "base",
        }),
      ),
    [workers],
  );

  const displayRole = (worker) => worker.current_role || "None";
  const displayRun = (worker) => {
    if (!worker.current_role) return "N/A";
    if (worker.current_run_name) return worker.current_run_name;
    return "N/A";
  };

  useEffect(() => {
    if (workers.length === 0) {
      setSelectedNodeName(null);
      return;
    }

    const stillExists = sortedWorkers.some((worker) => nodeNameFor(worker) === selectedNodeName);
    if (!stillExists) setSelectedNodeName(nodeNameFor(sortedWorkers[0]));
  }, [selectedNodeName, sortedWorkers, workers.length]);

  const selectedWorker = useMemo(
    () => sortedWorkers.find((worker) => nodeNameFor(worker) === selectedNodeName) || null,
    [selectedNodeName, sortedWorkers],
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
          Nodes
        </Typography>

        {workers.length === 0 ? (
          <EmptyStateCard
            title="No nodes registered"
            message="Start one or more run-node processes to inspect desired assignment, current role, and heartbeat."
          />
        ) : (
          <Stack spacing={2}>
            <Box sx={{ display: "flex", flexWrap: "wrap", gap: 2 }}>
              <Typography variant="body2" color="text.secondary">
                total nodes: <strong>{workers.length}</strong>
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

            <TableContainer component={Paper} variant="outlined">
              <Table size="small" aria-label="nodes table">
                <TableHead>
                  <TableRow>
                    <TableCell>Node</TableCell>
                    <TableCell>Run</TableCell>
                    <TableCell>Role</TableCell>
                    <TableCell>Last Seen</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {sortedWorkers.map((worker) => {
                    const nodeName = nodeNameFor(worker);
                    const selected = nodeName === selectedNodeName;
                    return (
                      <TableRow
                        key={nodeName}
                        hover
                        selected={selected}
                        onClick={() => setSelectedNodeName(nodeName)}
                        sx={{
                          cursor: "pointer",
                          "& .MuiTableCell-root": {
                            fontFamily: selected ? "monospace" : "inherit",
                          },
                        }}
                      >
                        <TableCell>{nodeName || "unknown"}</TableCell>
                        <TableCell>{displayRun(worker)}</TableCell>
                        <TableCell>{displayRole(worker)}</TableCell>
                        <TableCell>{formatDateTime(worker.last_seen, "-")}</TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </TableContainer>
          </Stack>
        )}
      </Paper>

      {selectedWorker ? (
        <WorkerDetailsPanel worker={selectedWorker} runs={runs} isConnected={isConnected} />
      ) : (
        <Alert severity="info">Select a node to view assignment and heartbeat details.</Alert>
      )}
    </>
  );
};

export default WorkersWorkspace;
