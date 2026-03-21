import {
  Alert,
  Box,
  Button,
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
import { assignNode, unassignNode } from "../services/api";

const WorkerDetailsPanel = ({ worker, runs = [] }) => {
  const nodeName = worker?.node_name || null;
  const { panelSpecs, panelStates, error } = useNodePanels({
    nodeName,
    pollMs: 3000,
  });
  const { authenticated, requireAuth } = useAuth();
  const [selectedRunId, setSelectedRunId] = useState("");
  const [selectedRole, setSelectedRole] = useState("evaluator");
  const [busy, setBusy] = useState(false);
  const [snackbar, setSnackbar] = useState(null);

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
              onClick={() =>
                requireAuth(async () => {
                  setBusy(true);
                  try {
                    await assignNode(nodeName, { runId: Number(selectedRunId), role: selectedRole });
                    setSnackbar({ message: "Assignment updated." });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to assign node." });
                    throw err;
                  } finally {
                    setBusy(false);
                  }
                })
              }
            >
              {authenticated ? "Assign" : "Log In To Assign"}
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                requireAuth(async () => {
                  setBusy(true);
                  try {
                    await unassignNode(nodeName);
                    setSnackbar({ message: "Node unassigned." });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to unassign node." });
                    throw err;
                  } finally {
                    setBusy(false);
                  }
                })
              }
            >
              {authenticated ? "Unassign" : "Log In To Unassign"}
            </Button>
          </Stack>
        </Stack>
      </Paper>
      <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} />
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
