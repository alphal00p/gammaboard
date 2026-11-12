import {
  Card,
  CardContent,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
} from "@mui/material";
import DisclosureField from "./common/DisclosureField";
import { formatCompactNumber } from "../utils/formatters";
import StatusChip from "./common/StatusChip";

const TaskQueuePanel = ({ tasks = [], selectedTaskId = null, onSelectTask = null, actions = null, renderTaskActions = null }) => {
  return (
    <>
      <DisclosureField section label={`${tasks.length} task${tasks.length === 1 ? "" : "s"}`}
        ariaLabel="Task queue" actions={actions} defaultOpen>
      <Card id="task-queue-table">
        <CardContent sx={{ p: 1, "&:last-child": { pb: 1 } }}>
          <TableContainer role="region" aria-label="Task queue table" tabIndex={0} sx={{ maxHeight: 240, overflow: "auto" }}>
            <Table size="small" stickyHeader>
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>State</TableCell>
                  <TableCell>Kind</TableCell>
                  <TableCell>Failure</TableCell>
                  <TableCell align="right">Goal</TableCell>
                  <TableCell align="right">Completed</TableCell>
                  <TableCell align="right">Allocated core-hours</TableCell>
                  {renderTaskActions && <TableCell align="right">Actions</TableCell>}
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
                      tabIndex={0}
                      aria-label={`Select task ${task.name || "Unnamed task"}`}
                      onKeyDown={(event) => { if (event.target === event.currentTarget && ["Enter", " "].includes(event.key)) { event.preventDefault(); onSelectTask?.(task.id); } }}
                      onClick={() => onSelectTask?.(task.id)}
                      sx={{
                        cursor: "pointer",
                        ...(task.state === "active" ? { backgroundColor: "action.hover" } : {}),
                      }}
                    >
                      <TableCell>{task.name || "Unnamed task"}</TableCell>
                      <TableCell><StatusChip status={task.state} /></TableCell>
                      <TableCell>{task.task_kind}</TableCell>
                      <TableCell>{task.failure_reason || "-"}</TableCell>
                      <TableCell align="right">
                        {Number.isFinite(Number(task.goal_label))
                          ? Number(task.goal_label).toLocaleString()
                          : task.goal_label}
                      </TableCell>
                      <TableCell align="right">
                        {Number(task.nr_completed_samples_including_children ?? 0).toLocaleString()}
                      </TableCell>
                      <TableCell align="right">
                        {formatCompactNumber(task.cpu_hours_including_children ?? 0)}
                      </TableCell>
                      {renderTaskActions && <TableCell align="right" onClick={(event) => event.stopPropagation()}>
                        {renderTaskActions(task)}
                      </TableCell>}
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
        </CardContent>
      </Card>
      </DisclosureField>
    </>
  );
};

export default TaskQueuePanel;
