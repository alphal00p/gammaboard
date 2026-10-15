import { Alert, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";

const TaskOutputPanel = ({
  runId,
  task,
  title = "Selected Task Output",
  onSelectRun = null,
}) => {
  const { panelSpecs, panelStates, panelValues, setPanelValue, error, lastUpdate } = useTaskOutput({
    runId,
    taskId: task?.id ?? null,
    pollMs: 2000,
    refreshKey: task?.state,
    panelLimit: 500,
  });

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  if (error) {
    return <Alert severity="error">{error}</Alert>;
  }

  if (panelSpecs.length === 0) return null;

  return (
    <>
    <Typography variant="caption" color="text.secondary">Output for {task.name} · updated {lastUpdate || "pending"}. Live sections refresh separately.</Typography>
    <PanelCollection
      title={title}
      panelSpecs={panelSpecs}
      panelStates={panelStates}
      panelValues={panelValues}
      onPanelValueChange={setPanelValue}
      onSelectRun={onSelectRun}
    />
    </>
  );
};

export default TaskOutputPanel;
