import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";

const TaskOutputPanel = ({ runId, task }) => {
  const { panelSpecs, panelStates, panelValues, setPanelValue, error } = useTaskOutput({
    runId,
    taskId: task?.id ?? null,
    pollMs: 3000,
    panelLimit: 500,
  });

  if (!task) {
    return <Alert severity="info">Select a task to inspect its output panels.</Alert>;
  }

  if (error) {
    return (
      <Box>
        <Alert severity="error">{error}</Alert>
      </Box>
    );
  }

  return (
    <Box>
      <PanelCollection
        title="Selected Task Output"
        panelSpecs={panelSpecs}
        panelStates={panelStates}
        panelValues={panelValues}
        onPanelValueChange={setPanelValue}
      />
    </Box>
  );
};

export default TaskOutputPanel;
