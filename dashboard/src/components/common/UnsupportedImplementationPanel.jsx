import { Card, CardContent, Typography } from "@mui/material";

const UnsupportedImplementationPanel = ({ kind, implementation }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
        Implementation Details
      </Typography>
      <Typography variant="body2" color="text.secondary">
        No custom {kind} panel available for <strong>{implementation || "unknown"}</strong>.
      </Typography>
    </CardContent>
  </Card>
);

export default UnsupportedImplementationPanel;
