import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useRunPanels } from "../hooks/useRunPanels";

const TOP_LEVEL_PANEL_IDS = ["run_identity", "run_progress"];

const RunInfo = ({ runId }) => {
  const { panelSpecs, panelStates, error } = useRunPanels({ runId, pollMs: 5000 });

  if (runId == null) return null;

  if (error) {
    return (
      <Box sx={{ mb: 3 }}>
        <Alert severity="error">{error}</Alert>
      </Box>
    );
  }

  const topLevelSpecs = panelSpecs.filter((panel) => TOP_LEVEL_PANEL_IDS.includes(panel?.panel_id));
  const topLevelStates = panelStates.filter((panel) => TOP_LEVEL_PANEL_IDS.includes(panel?.panel_id));

  return <PanelCollection title="Run Summary" panelSpecs={topLevelSpecs} panelStates={topLevelStates} />;
};

export default RunInfo;
