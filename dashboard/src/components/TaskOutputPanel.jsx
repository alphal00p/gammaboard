import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useTaskOutput } from "../hooks/useTaskOutput";

const TaskOutputPanel = ({
  runId,
  task,
  includePanelIds = null,
  excludePanelIds = null,
  title = "Selected Task Output",
}) => {
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

  const includeSet = includePanelIds ? new Set(includePanelIds) : null;
  const excludeSet = excludePanelIds ? new Set(excludePanelIds) : null;
  const filteredSpecs = panelSpecs.filter((panel) => {
    if (includeSet && !includeSet.has(panel?.panel_id)) return false;
    if (excludeSet && excludeSet.has(panel?.panel_id)) return false;
    return true;
  });
  const filteredStates = panelStates.filter((panel) => {
    if (includeSet && !includeSet.has(panel?.panel_id)) return false;
    if (excludeSet && excludeSet.has(panel?.panel_id)) return false;
    return true;
  });
  const filteredValues =
    panelValues == null
      ? panelValues
      : Object.fromEntries(
          Object.entries(panelValues).filter(([panelId]) => {
            if (includeSet && !includeSet.has(panelId)) return false;
            if (excludeSet && excludeSet.has(panelId)) return false;
            return true;
          }),
        );

  if (filteredSpecs.length === 0) return null;

  return (
    <Box>
      <PanelCollection
        title={title}
        panelSpecs={filteredSpecs}
        panelStates={filteredStates}
        panelValues={filteredValues}
        onPanelValueChange={setPanelValue}
      />
    </Box>
  );
};

export default TaskOutputPanel;
