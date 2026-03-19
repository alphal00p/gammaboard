import {
  Alert,
  Box,
  Card,
  CardContent,
  TableContainer,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import {
  formatTaskSnapshotRef,
  formatTaskSpawnOrigin,
  getCurrentTask,
  getTaskKindLabel,
  getTaskTargetLabel,
} from "../utils/tasks";

const TaskQueuePanel = ({ tasks = [], selectedTaskId = null, onSelectTask = null }) => {
  const currentTask = getCurrentTask(tasks);

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Task Queue
      </Typography>
      <Card>
        <CardContent>
          {currentTask ? (
            <Box sx={{ mb: 2 }}>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                current: #{currentTask.sequence_nr} {getTaskKindLabel(currentTask)} ({currentTask.state})
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                target_samples: {getTaskTargetLabel(currentTask)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                produced_samples: {Number(currentTask.nr_produced_samples || 0).toLocaleString()}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                completed_samples: {Number(currentTask.nr_completed_samples || 0).toLocaleString()}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                start_from: {formatTaskSnapshotRef(currentTask.task?.start_from)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                spawned_from: {formatTaskSpawnOrigin(currentTask)}
              </Typography>
            </Box>
          ) : (
            <Alert severity="info" sx={{ mb: 2 }}>
              No task is currently active or pending.
            </Alert>
          )}

          <TableContainer>
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Seq</TableCell>
                  <TableCell>State</TableCell>
                  <TableCell>Task</TableCell>
                  <TableCell>Start From</TableCell>
                  <TableCell>Spawned From</TableCell>
                  <TableCell align="right">Target</TableCell>
                  <TableCell align="right">Produced</TableCell>
                  <TableCell align="right">Completed</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {tasks.map((task) => {
                  const isSelected = task.id === selectedTaskId;
                  return (
                    <TableRow
                      key={task.id}
                      hover
                      selected={isSelected}
                      onClick={() => onSelectTask?.(task.id)}
                      sx={{
                        cursor: "pointer",
                        ...(task.state === "active" ? { backgroundColor: "action.hover" } : {}),
                      }}
                    >
                      <TableCell>{task.sequence_nr}</TableCell>
                      <TableCell>{task.state}</TableCell>
                      <TableCell>{getTaskKindLabel(task)}</TableCell>
                      <TableCell>{formatTaskSnapshotRef(task.task?.start_from)}</TableCell>
                      <TableCell>{formatTaskSpawnOrigin(task)}</TableCell>
                      <TableCell align="right">{getTaskTargetLabel(task)}</TableCell>
                      <TableCell align="right">{Number(task.nr_produced_samples || 0).toLocaleString()}</TableCell>
                      <TableCell align="right">{Number(task.nr_completed_samples || 0).toLocaleString()}</TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        </CardContent>
      </Card>
    </Box>
  );
};

export default TaskQueuePanel;
