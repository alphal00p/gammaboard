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
  const taskList = asTaskList(sourceTasks).filter((task) => Number.isFinite(Number(task.latest_stage_snapshot_id)));

  useEffect(() => {
    if (!open) return;
    setNewName(initialName);
  }, [initialName, open]);

  useEffect(() => {
    if (!open) return;
    if (taskList.length === 0) {
      setFromSnapshotId("");
      return;
    }
    if (taskList.some((task) => String(task.latest_stage_snapshot_id) === String(fromSnapshotId))) {
      return;
    }
    const completedTask = taskList.find((task) => task.state === "completed");
    setFromSnapshotId(String((completedTask ?? taskList[0]).latest_stage_snapshot_id));
  }, [fromSnapshotId, open, taskList]);

  const selectedRunValue = sourceRunId == null ? "" : String(sourceRunId);

  const selectedTask = useMemo(
    () => taskList.find((task) => String(task.latest_stage_snapshot_id) === String(fromSnapshotId)) ?? null,
    [fromSnapshotId, taskList],
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
              Create a new run from the latest stored stage snapshot of a selected task.
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
              label="From Task"
              value={fromSnapshotId}
              onChange={(event) => setFromSnapshotId(event.target.value)}
              disabled={taskList.length === 0}
              helperText={
                selectedTask
                  ? `${getTaskKindLabel(selectedTask)} task #${selectedTask.id} -> snapshot #${selectedTask.latest_stage_snapshot_id}`
                  : "No tasks with stored stage snapshots available"
              }
            >
              {taskList.map((task) => (
                <MenuItem key={task.id} value={task.latest_stage_snapshot_id}>
                  #{task.id} · {task.state} · {getTaskKindLabel(task)} · snapshot #{task.latest_stage_snapshot_id}
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
