import { Box, Typography } from "@mui/material";
import JsonFallback from "../JsonFallback";

const EnginePanelLayout = ({ title, genericPanel, customPanel, jsonTitle, jsonData }) => (
  <Box sx={{ mb: 3 }}>
    <Typography variant="h6" gutterBottom>
      {title}
    </Typography>
    {genericPanel}
    <Box sx={{ mt: 2 }}>{customPanel}</Box>
    <Box sx={{ mt: 2 }}>
      <JsonFallback title={jsonTitle} data={jsonData} />
    </Box>
  </Box>
);

export default EnginePanelLayout;
