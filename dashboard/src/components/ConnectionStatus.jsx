import { Box, Chip, Typography } from "@mui/material";
import WifiIcon from "@mui/icons-material/Wifi";
import WifiOffIcon from "@mui/icons-material/WifiOff";

const ConnectionStatus = ({ isConnected, lastUpdate, serverName = "local" }) => {
  const normalizedServerName = typeof serverName === "string" && serverName.trim() ? serverName.trim() : "local";
  return (
    <Box sx={{ mb: 3, display: "flex", alignItems: "center", gap: 2, flexWrap: "wrap" }}>
      <Chip
        icon={isConnected ? <WifiIcon /> : <WifiOffIcon />}
        label={isConnected ? `Connected to ${normalizedServerName}` : `Disconnected from ${normalizedServerName}`}
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
