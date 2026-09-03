import { Alert, Box, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";

const RunConfigPanel = ({ panelResponse = null }) => (
  <Box sx={{ mb: 3 }}>
    <Typography variant="h6" gutterBottom>
      Effective Engine Configuration
    </Typography>
    {panelResponse?.panelSpecs?.length ? (
      <PanelCollection panelSpecs={panelResponse.panelSpecs} panelStates={panelResponse.panelStates || []} />
    ) : (
      <Alert severity="info">The current run stage has no evaluator or sampler configuration.</Alert>
    )}
  </Box>
);

export default RunConfigPanel;
