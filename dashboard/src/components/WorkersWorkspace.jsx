import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Paper,
  Snackbar,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import WorkerDetailsPanel from "./WorkerDetailsPanel";
import EmptyStateCard from "./common/EmptyStateCard";
import { formatDateTime } from "../utils/formatters";
import { compareNodesByName, nodeNameOf } from "../utils/nodes";
import { useAuth } from "../auth/AuthProvider";
import { useNodeLaunchRequests } from "../hooks/useNodeLaunchRequests";
import { autoRunNodes, restartDatabase, shutdownControlProcess, stopAllNodes } from "../services/api";

const DEFAULT_NODE_LAUNCH_TOML = `[[groups]]
count = 1
name_prefix = "gpu"
max_start_failures = 6
config = { gres = "gpu:1" }

[[groups]]
count = 9
name_prefix = "cpu"
max_start_failures = 3
config = {}
`;

const WorkersWorkspace = ({ workers, runs, isConnected, lastUpdate, error }) => {
  const { authenticated } = useAuth();
  const [selectedNodeName, setSelectedNodeName] = useState(null);
  const [launchToml, setLaunchToml] = useState(DEFAULT_NODE_LAUNCH_TOML);
  const [startingNodes, setStartingNodes] = useState(false);
  const [stoppingAllNodes, setStoppingAllNodes] = useState(false);
  const [restartDbOpen, setRestartDbOpen] = useState(false);
  const [restartingDb, setRestartingDb] = useState(false);
  const [shutdownControlOpen, setShutdownControlOpen] = useState(false);
  const [shuttingDownControl, setShuttingDownControl] = useState(false);
  const [snackbar, setSnackbar] = useState(null);
  const launchRequestsData = useNodeLaunchRequests({ enabled: authenticated });
  const sortedWorkers = useMemo(() => [...workers].sort(compareNodesByName), [workers]);

  const displayRole = (worker) => worker.current_role || "None";
  const displayRun = (worker) => {
    if (!worker.current_role) return "N/A";
    if (worker.current_run_name) return worker.current_run_name;
    return "N/A";
  };
  const displayCapabilities = (worker) => {
    const entries = Object.entries(worker?.capabilities || {});
    if (entries.length === 0) return "-";
    return entries
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, value]) => `${key}=${value}`)
      .join(", ");
  };

  useEffect(() => {
    if (workers.length === 0) {
      setSelectedNodeName(null);
      return;
    }

    const stillExists = sortedWorkers.some((worker) => nodeNameOf(worker) === selectedNodeName);
    if (!stillExists) setSelectedNodeName(nodeNameOf(sortedWorkers[0]));
  }, [selectedNodeName, sortedWorkers, workers.length]);

  const selectedWorker = useMemo(
    () => sortedWorkers.find((worker) => nodeNameOf(worker) === selectedNodeName) || null,
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
          Node Management
        </Typography>
        {authenticated ? (
          <Stack spacing={1} sx={{ mb: 2 }}>
            <TextField
              label="Node Launch TOML"
              value={launchToml}
              onChange={(event) => setLaunchToml(event.target.value)}
              multiline
              minRows={8}
              maxRows={16}
              fullWidth
            />
            <Button
              sx={{ width: "fit-content" }}
              variant="outlined"
              disabled={startingNodes}
              onClick={async () => {
                if (!String(launchToml || "").trim()) {
                  setSnackbar({ message: "Launch TOML must not be empty.", severity: "error" });
                  return;
                }
                setStartingNodes(true);
                try {
                  const response = await autoRunNodes({ toml: launchToml });
                  const requestId = response?.request?.id;
                  const started = Number(response?.started ?? 0);
                  setSnackbar({
                    message: `Created node launch request ${requestId ?? ""}; submitted ${started} node${started === 1 ? "" : "s"}.`,
                    severity: "success",
                  });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to request nodes.", severity: "error" });
                } finally {
                  setStartingNodes(false);
                }
              }}
            >
              Request Nodes
            </Button>
          </Stack>
        ) : null}

        <Box sx={{ mb: 3 }}>
          <Typography variant="subtitle1" gutterBottom>
            Node Startup Queue
          </Typography>
          {!authenticated ? (
            <EmptyStateCard title="Login required" message="Authenticate to inspect and create node launch requests." />
          ) : launchRequestsData.error ? (
            <Alert severity="error" sx={{ mb: 2 }}>
              Failed to fetch node launch requests.
            </Alert>
          ) : launchRequestsData.launchRequests.length === 0 ? (
            <EmptyStateCard title="No launch requests" message="Node start requests will appear here." />
          ) : (
            <TableContainer component={Paper} variant="outlined">
              <Table size="small" aria-label="node launch requests table">
                <TableHead>
                  <TableRow>
                    <TableCell>ID</TableCell>
                    <TableCell>State</TableCell>
                    <TableCell>Backend</TableCell>
                    <TableCell>Count</TableCell>
                    <TableCell>Submitted</TableCell>
                    <TableCell>Created</TableCell>
                    <TableCell>Error</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {launchRequestsData.launchRequests.map((request) => (
                    <TableRow key={request.id}>
                      <TableCell>{request.id}</TableCell>
                      <TableCell>{request.state}</TableCell>
                      <TableCell>{request.backend}</TableCell>
                      <TableCell>{request.requested_count}</TableCell>
                      <TableCell>{request.started_count}</TableCell>
                      <TableCell>{formatDateTime(request.created_at, "-")}</TableCell>
                      <TableCell>{request.error || "-"}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </Box>

        <Typography variant="subtitle1" gutterBottom>
          Live Nodes
        </Typography>

        {workers.length === 0 ? (
          <EmptyStateCard
            title="No nodes registered"
            message="Start one or more node run processes to inspect desired assignment, current role, and heartbeat."
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
                    <TableCell>Capabilities</TableCell>
                    <TableCell>Last Seen</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {sortedWorkers.map((worker) => {
                    const nodeName = nodeNameOf(worker);
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
                        <TableCell>{displayCapabilities(worker)}</TableCell>
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
      {authenticated ? (
        <Paper variant="outlined" sx={{ p: 2, mt: 3, borderColor: "error.light" }}>
          <Typography variant="h6" color="error" gutterBottom>
            Danger Zone
          </Typography>
          <Stack direction="row" spacing={1} sx={{ flexWrap: "wrap", gap: 1 }}>
            <Button
              color="error"
              variant="outlined"
              disabled={stoppingAllNodes}
              onClick={async () => {
                setStoppingAllNodes(true);
                try {
                  const response = await stopAllNodes();
                  const rows = Number(response?.rows_updated ?? 0);
                  setSnackbar({
                    message: `Requested shutdown for ${rows} node${rows === 1 ? "" : "s"}.`,
                    severity: "success",
                  });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to stop all nodes.", severity: "error" });
                } finally {
                  setStoppingAllNodes(false);
                }
              }}
            >
              Stop All Nodes
            </Button>
            <Button color="warning" variant="outlined" disabled={restartingDb} onClick={() => setRestartDbOpen(true)}>
              Reset DB
            </Button>
            <Button
              color="error"
              variant="contained"
              disabled={shuttingDownControl}
              onClick={() => setShutdownControlOpen(true)}
            >
              Kill Control Process
            </Button>
          </Stack>
          <Divider sx={{ mt: 2 }} />
        </Paper>
      ) : null}
      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
      <Dialog
        open={restartDbOpen}
        onClose={() => (restartingDb ? null : setRestartDbOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Reset Database?</DialogTitle>
        <DialogContent>
          This will delete local database state and recreate it from migrations. Running nodes and run execution will be
          interrupted and existing local runs/data will be lost. Do you want to continue?
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setRestartDbOpen(false)} disabled={restartingDb}>
            Cancel
          </Button>
          <Button
            color="warning"
            variant="contained"
            disabled={restartingDb}
            onClick={async () => {
              setRestartingDb(true);
              try {
                await restartDatabase();
                setRestartDbOpen(false);
                setSnackbar({ message: "Database reset.", severity: "success" });
              } catch (err) {
                setSnackbar({ message: err?.message || "Failed to reset database.", severity: "error" });
              } finally {
                setRestartingDb(false);
              }
            }}
          >
            Confirm Reset
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={shutdownControlOpen}
        onClose={() => (shuttingDownControl ? null : setShutdownControlOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Kill Control Process?</DialogTitle>
        <DialogContent>
          This will ask the running backend process to exit. The dashboard and API will go offline until the deployment is
          restarted.
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setShutdownControlOpen(false)} disabled={shuttingDownControl}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={shuttingDownControl}
            onClick={async () => {
              setShuttingDownControl(true);
              try {
                await shutdownControlProcess();
                setShutdownControlOpen(false);
                setSnackbar({ message: "Control shutdown requested.", severity: "success" });
              } catch (err) {
                setSnackbar({ message: err?.message || "Failed to shut down control process.", severity: "error" });
                setShuttingDownControl(false);
              }
            }}
          >
            Kill Control Process
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
};

export default WorkersWorkspace;
