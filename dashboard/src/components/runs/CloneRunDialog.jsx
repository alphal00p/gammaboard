import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { asTaskList, getTaskKindLabel } from "../../utils/tasks";

const CloneRunDialog = ({
  open,
  runs,
  sourceRunId,
  setSourceRunId,
  sourceTasks,
  initialName,
  busy = false,
  error = null,
  onClose,
  onSubmit,
}) => {
  const [newName, setNewName] = useState(initialName);
  const [fromSnapshotId, setFromSnapshotId] = useState("");
  const taskList = asTaskList(sourceTasks);
  const selectedRun = useMemo(
    () => runs.find((run) => String(run.run_id) === String(sourceRunId)) ?? null,
    [runs, sourceRunId],
  );
  const rootSnapshotId = useMemo(() => {
    const runRootSnapshotId = Number(selectedRun?.root_stage_snapshot_id);
    if (Number.isFinite(runRootSnapshotId)) {
      return runRootSnapshotId;
    }
    return (
      taskList.map((task) => Number(task.root_stage_snapshot_id)).find((snapshotId) => Number.isFinite(snapshotId)) ??
      null
    );
  }, [selectedRun, taskList]);
  const snapshotTaskList = useMemo(
    () => taskList.filter((task) => Number.isFinite(Number(task.latest_stage_snapshot_id))),
    [taskList],
  );
  const options = useMemo(() => {
    const optionList = [];
    if (Number.isFinite(Number(rootSnapshotId))) {
      optionList.push({
        value: String(rootSnapshotId),
        label: `Initial state (snapshot #${rootSnapshotId})`,
        helper: "Root stage snapshot",
      });
    }
    for (const task of snapshotTaskList) {
      optionList.push({
        value: String(task.latest_stage_snapshot_id),
        label: `${task.name || `task-${task.id}`} · ${task.state} · ${getTaskKindLabel(task)} · snapshot #${task.latest_stage_snapshot_id}`,
        helper: `${task.name || `task-${task.id}`} -> snapshot #${task.latest_stage_snapshot_id}`,
      });
    }
    return optionList;
  }, [rootSnapshotId, snapshotTaskList]);

  useEffect(() => {
    if (!open) return;
    setNewName(initialName);
  }, [initialName, open]);

  useEffect(() => {
    if (!open) return;
    if (options.length === 0) {
      setFromSnapshotId("");
      return;
    }
    if (options.some((option) => option.value === String(fromSnapshotId))) {
      return;
    }
    const completedTask = snapshotTaskList.find((task) => task.state === "completed");
    setFromSnapshotId(String(completedTask?.latest_stage_snapshot_id ?? options[0].value));
  }, [fromSnapshotId, open, options, snapshotTaskList]);

  const selectedRunValue = sourceRunId == null ? "" : String(sourceRunId);

  const selectedOption = useMemo(
    () => options.find((option) => option.value === String(fromSnapshotId)) ?? null,
    [fromSnapshotId, options],
  );

  const handleClose = () => {
    if (busy) return;
    onClose();
  };

  const handleSubmit = async (event) => {
    event.preventDefault();
    await onSubmit({
      sourceRunId,
      fromSnapshotId: Number(fromSnapshotId),
      newName,
    });
  };

  return (
    <Dialog open={open} onClose={handleClose} fullWidth maxWidth="sm">
      <form onSubmit={handleSubmit}>
        <DialogTitle>Clone Run</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 1 }}>
            <Typography variant="body2" color="text.secondary">
              Create a new run from a stored stage snapshot, including the initial root snapshot.
            </Typography>
            <TextField
              select
              fullWidth
              label="Source Run"
              value={selectedRunValue}
              onChange={(event) => setSourceRunId(Number(event.target.value))}
            >
              {runs.map((run) => (
                <MenuItem key={run.run_id} value={run.run_id}>
                  {run.run_name} (#{run.run_id})
                </MenuItem>
              ))}
            </TextField>
            <TextField
              select
              fullWidth
              label="From Snapshot"
              value={fromSnapshotId}
              onChange={(event) => setFromSnapshotId(event.target.value)}
              disabled={options.length === 0}
              helperText={selectedOption?.helper ?? "No stored stage snapshots available"}
            >
              {options.map((option) => (
                <MenuItem key={option.value} value={option.value}>
                  {option.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              autoFocus
              fullWidth
              label="New Run Name"
              value={newName}
              onChange={(event) => setNewName(event.target.value)}
            />
            {error ? <Alert severity="error">{error}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={handleClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="contained"
            disabled={busy || sourceRunId == null || !fromSnapshotId || !newName.trim()}
          >
            Clone Run
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  );
};

export default CloneRunDialog;
