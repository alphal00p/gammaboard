import {
  Card,
  CardContent,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { formatTaskSnapshotRef, formatTaskSpawnOrigin, getTaskKindLabel, getTaskTargetLabel } from "../utils/tasks";

const TaskQueuePanel = ({ tasks = [], selectedTaskId = null, onSelectTask = null }) => {
  return (
    <>
      <Typography variant="h6" gutterBottom>
        Task Queue
      </Typography>
      <Card>
        <CardContent>
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
    </>
  );
};

export default TaskQueuePanel;
