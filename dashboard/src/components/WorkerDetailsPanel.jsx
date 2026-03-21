import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControl,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Snackbar,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import PanelCollection from "./panels/PanelCollection";
import { useNodePanels } from "../hooks/useNodePanels";
import { assignNode, stopNode, unassignNode } from "../services/api";

const WorkerDetailsPanel = ({ worker, runs = [] }) => {
  const nodeName = worker?.node_name || null;
  const { panelSpecs, panelStates, error } = useNodePanels({
    nodeName,
    pollMs: 3000,
  });
  const { authenticated } = useAuth();
  const [selectedRunId, setSelectedRunId] = useState("");
  const [selectedRole, setSelectedRole] = useState("evaluator");
  const [busy, setBusy] = useState(false);
  const [snackbar, setSnackbar] = useState(null);
  const [confirmStopOpen, setConfirmStopOpen] = useState(false);

  const runOptions = useMemo(() => runs.filter((run) => Number.isFinite(Number(run?.run_id))), [runs]);

  useEffect(() => {
    setSelectedRunId(worker?.desired_run_id ?? "");
    setSelectedRole(worker?.desired_role ?? "evaluator");
  }, [worker?.desired_role, worker?.desired_run_id]);

  if (!worker) return null;

  return (
    <Box>
      <Typography variant="h6" gutterBottom>
        Node Details
      </Typography>
      {error ? <Alert severity="error">{error}</Alert> : null}
      {authenticated ? (
        <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={2} alignItems={{ md: "center" }}>
            <FormControl size="small" sx={{ minWidth: 220 }}>
              <InputLabel id="assign-run-label">Run</InputLabel>
              <Select
                labelId="assign-run-label"
                label="Run"
                value={selectedRunId}
                onChange={(event) => setSelectedRunId(Number(event.target.value))}
              >
                {runOptions.map((run) => (
                  <MenuItem key={run.run_id} value={run.run_id}>
                    {run.run_name} (#{run.run_id})
                  </MenuItem>
                ))}
              </Select>
            </FormControl>
            <FormControl size="small" sx={{ minWidth: 180 }}>
              <InputLabel id="assign-role-label">Role</InputLabel>
              <Select
                labelId="assign-role-label"
                label="Role"
                value={selectedRole}
                onChange={(event) => setSelectedRole(event.target.value)}
              >
                <MenuItem value="evaluator">Evaluator</MenuItem>
                <MenuItem value="sampler_aggregator">Sampler Aggregator</MenuItem>
              </Select>
            </FormControl>
            <Stack direction="row" spacing={1}>
              <Button
                variant="contained"
                disabled={busy || !selectedRunId}
                onClick={async () => {
                  setBusy(true);
                  try {
                    await assignNode(nodeName, { runId: Number(selectedRunId), role: selectedRole });
                    setSnackbar({ message: "Assignment updated." });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to assign node." });
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Assign
              </Button>
              <Button
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    await unassignNode(nodeName);
                    setSnackbar({ message: "Node unassigned." });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to unassign node." });
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Unassign
              </Button>
              <Button color="error" disabled={busy} onClick={() => setConfirmStopOpen(true)}>
                Stop Node
              </Button>
            </Stack>
          </Stack>
        </Paper>
      ) : null}
      <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} />
      <Dialog open={confirmStopOpen} onClose={() => (busy ? null : setConfirmStopOpen(false))} maxWidth="sm" fullWidth>
        <DialogTitle>Stop Node?</DialogTitle>
        <DialogContent>
          This will request a complete shutdown of <strong>{nodeName}</strong>. Do you really want to continue? This
          will permanently shut this down without a way to restart from the webapp.
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConfirmStopOpen(false)} disabled={busy}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await stopNode(nodeName);
                setConfirmStopOpen(false);
                setSnackbar({ message: "Shutdown requested for node." });
              } catch (err) {
                setSnackbar({ message: err?.message || "Failed to stop node." });
              } finally {
                setBusy(false);
              }
            }}
          >
            Confirm Stop
          </Button>
        </DialogActions>
      </Dialog>
      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
    </Box>
  );
};

export default WorkerDetailsPanel;
