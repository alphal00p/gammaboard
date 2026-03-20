import { Alert, Box, Typography } from "@mui/material";
import PanelCollection from "./panels/PanelCollection";
import { useNodePanels } from "../hooks/useNodePanels";

const WorkerDetailsPanel = ({ worker }) => {
  const nodeId = worker?.node_id || worker?.worker_id || null;
  const { panelSpecs, panelStates, error } = useNodePanels({
    nodeId,
    pollMs: 3000,
  });

  if (!worker) return null;

  return (
    <Box>
      <Typography variant="h6" gutterBottom>
        Node Details
      </Typography>
      {error ? <Alert severity="error">{error}</Alert> : null}
      <PanelCollection panelSpecs={panelSpecs} panelStates={panelStates} />
    </Box>
  );
};

export default WorkerDetailsPanel;
