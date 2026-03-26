import {
  Card,
  CardContent,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { formatTaskSourceRef, getTaskKindLabel, getTaskTargetLabel } from "../utils/tasks";

const TaskQueuePanel = ({ tasks = [], selectedTaskId = null, onSelectTask = null, actions = null }) => {
  return (
    <>
      <Stack direction="row" spacing={1} alignItems="center" justifyContent="space-between" sx={{ mb: 1 }}>
        <Typography variant="h6">Task Queue</Typography>
        {actions}
      </Stack>
      <Card>
        <CardContent>
          <TableContainer>
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>State</TableCell>
                  <TableCell>Task</TableCell>
                  <TableCell>Sampler Source</TableCell>
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
                      <TableCell>{task.name || "Unnamed task"}</TableCell>
                      <TableCell>{task.state}</TableCell>
                      <TableCell>{getTaskKindLabel(task)}</TableCell>
                      <TableCell>{formatTaskSourceRef(task)}</TableCell>
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
