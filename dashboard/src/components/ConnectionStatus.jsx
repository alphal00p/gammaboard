import { Box, Typography } from "@mui/material";
import WifiIcon from "@mui/icons-material/Wifi";
import WifiOffIcon from "@mui/icons-material/WifiOff";
import StatusChip from "./common/StatusChip";

const ConnectionStatus = ({ isConnected, lastUpdate, serverName = "local" }) => {
  const normalizedServerName = typeof serverName === "string" && serverName.trim() ? serverName.trim() : "local";
  return (
    <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, flexWrap: "wrap", minWidth: 0, "& .MuiChip-root": { maxWidth: "100%" } }}>
      <StatusChip
        status={isConnected ? "connected" : "disconnected"}
        icon={isConnected ? <WifiIcon /> : <WifiOffIcon />}
        label={isConnected ? `Connected to ${normalizedServerName}` : `Disconnected from ${normalizedServerName}`}
      />

      {lastUpdate && (
        <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "nowrap", fontVariantNumeric: "tabular-nums" }}>
          Last update: {lastUpdate}
        </Typography>
      )}
    </Box>
  );
};

export default ConnectionStatus;
