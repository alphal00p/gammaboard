import { Box, Chip, Typography } from "@mui/material";
import { WifiOff as WifiOffIcon, Wifi as WifiIcon } from "@mui/icons-material";

const ConnectionStatus = ({ isConnected, lastUpdate }) => {
  return (
    <Box sx={{ mb: 3, display: "flex", alignItems: "center", gap: 2, flexWrap: "wrap" }}>
      <Chip
        icon={isConnected ? <WifiIcon /> : <WifiOffIcon />}
        label={isConnected ? "Connected" : "Disconnected"}
        color={isConnected ? "success" : "error"}
        variant="outlined"
      />

      {lastUpdate && (
        <Typography variant="body2" color="text.secondary">
          Last update: {lastUpdate}
        </Typography>
      )}
    </Box>
  );
};

export default ConnectionStatus;
