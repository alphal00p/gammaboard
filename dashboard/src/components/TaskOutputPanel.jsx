import { Alert, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";
import { formatTaskSnapshotRef, formatTaskSpawnOrigin } from "../utils/tasks";

const TaskOutputPanel = ({ runId, task }) => {
  const { output, historyItems } = useTaskOutput({
    runId,
    taskId: task?.id ?? null,
    pollMs: 3000,
    historyLimit: 500,
  });

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  return (
    <>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        Selected task: #{task.sequence_nr} {task.task?.kind || "unknown"} ({task.state})
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        start_from={formatTaskSnapshotRef(task.task?.start_from)} spawned_from={formatTaskSpawnOrigin(task)}
      </Typography>
      <PanelCollection
        title="Selected Task Output"
        descriptors={output?.panels || []}
        currentPanels={output?.current || []}
        historyItems={historyItems}
      />
    </>
  );
};

export default TaskOutputPanel;
