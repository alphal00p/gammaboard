import { Paper, Typography } from "@mui/material";

const EmptyStateCard = ({ title, message }) => (
  <Paper variant="outlined" sx={{ p: 3 }}>
    <Typography variant="h6" gutterBottom>
      {title}
    </Typography>
    <Typography variant="body2" color="text.secondary">
      {message}
    </Typography>
  </Paper>
);

export default EmptyStateCard;
