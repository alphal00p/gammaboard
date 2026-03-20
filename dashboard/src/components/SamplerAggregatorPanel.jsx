import { Alert, Box, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";

const SamplerAggregatorPanel = ({ panelResponse = null }) => (
  <Box sx={{ mb: 3 }}>
    <Typography variant="h6" gutterBottom>
      Sampler Aggregator
    </Typography>
    {panelResponse?.panelSpecs?.length ? (
      <PanelCollection panelSpecs={panelResponse.panelSpecs} panelStates={panelResponse.panelStates || []} />
    ) : (
      <Alert severity="info">No sampler aggregator is configured for this run.</Alert>
    )}
  </Box>
);

export default SamplerAggregatorPanel;
