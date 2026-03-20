import { Alert, Box } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useRunPanels } from "../hooks/useRunPanels";

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

  return <PanelCollection title="Run Info" panelSpecs={panelSpecs} panelStates={panelStates} />;
};

export default RunInfo;
