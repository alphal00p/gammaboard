import { Box, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";

const EvaluatorPanel = ({ panelResponse = null }) => (
  <Box sx={{ mb: 3 }}>
    <Typography variant="h6" gutterBottom>
      Evaluator
    </Typography>
    <PanelCollection panelSpecs={panelResponse?.panelSpecs || []} panelStates={panelResponse?.panelStates || []} />
  </Box>
);

export default EvaluatorPanel;
