import { Alert, Box, Card, CardContent, Grid, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";
import { formatTaskSnapshotRef, formatTaskSpawnOrigin, getTaskKindLabel, getTaskTargetLabel } from "../utils/tasks";

const fmtInt = (value) => (Number.isFinite(Number(value)) ? Number(value).toLocaleString() : "0");

const TaskMetadataPanel = ({ task }) => (
  <Card sx={{ mb: 2 }}>
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
        Selected Task
      </Typography>
      <Grid container spacing={1.5}>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            identity
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            #{task.sequence_nr} {getTaskKindLabel(task)}
          </Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            state
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {task.state}
          </Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            target
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {getTaskTargetLabel(task)}
          </Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            progress
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {fmtInt(task.nr_completed_samples)} / {fmtInt(task.nr_produced_samples)}
          </Typography>
        </Grid>
        <Grid item xs={12} md={6}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            start_from
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {formatTaskSnapshotRef(task.task?.start_from)}
          </Typography>
        </Grid>
        <Grid item xs={12} md={6}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            spawned_from
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {formatTaskSpawnOrigin(task)}
          </Typography>
        </Grid>
      </Grid>
    </CardContent>
  </Card>
);

const TaskOutputPanel = ({ runId, task }) => {
  const { panelSpecs, panelStates, error } = useTaskOutput({
    runId,
    taskId: task?.id ?? null,
    pollMs: 3000,
    historyLimit: 500,
  });

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  if (error) {
    return (
      <Box>
        <TaskMetadataPanel task={task} />
        <Alert severity="error">{error}</Alert>
      </Box>
    );
  }

  return (
    <Box>
      <TaskMetadataPanel task={task} />
      <PanelCollection title="Selected Task Output" panelSpecs={panelSpecs} panelStates={panelStates} />
    </Box>
  );
};

export default TaskOutputPanel;
