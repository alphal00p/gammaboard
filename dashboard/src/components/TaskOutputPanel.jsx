import { Alert, Box, Card, CardContent, Typography } from "@mui/material";
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
      <Card sx={{ mb: 2 }}>
        <CardContent>
          <Typography variant="h6" sx={{ mb: 1 }}>
            Task TOML
          </Typography>
          <Box
            component="pre"
            sx={{
              m: 0,
              overflowX: "auto",
              whiteSpace: "pre-wrap",
              fontFamily: "monospace",
              fontSize: 13,
            }}
          >
            {task.task_toml || "# task TOML unavailable"}
          </Box>
        </CardContent>
      </Card>
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
