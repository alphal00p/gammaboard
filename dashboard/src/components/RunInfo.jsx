import { Alert, Box, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useRunPanels } from "../hooks/useRunPanels";

const RunInfo = ({ runId, refreshKey }) => {
  const { panelSpecs, panelStates, error, lastUpdate } = useRunPanels({ runId, refreshKey });

  if (runId == null) return null;

  if (error) {
    return (
      <Box sx={{ mb: 3 }}>
        <Alert severity="error">{error}</Alert>
      </Box>
    );
  }

  if (!Array.isArray(panelSpecs) || panelSpecs.length === 0) {
    return null;
  }

  return <><Typography variant="caption" color="text.secondary">Run totals across tasks · updated {lastUpdate || "pending"}</Typography>
    <PanelCollection title="Run Summary" panelSpecs={panelSpecs} panelStates={panelStates} /></>;
};

export default RunInfo;
