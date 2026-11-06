import { Chip } from "@mui/material";

const statusColors = {
  active: "info", running: "info", starting: "info", submitted: "info",
  completed: "success", fulfilled: "success", connected: "success", ready: "success",
  paused: "warning", draining: "warning", stopping: "warning",
  failed: "error", disconnected: "error", lost: "error", stale: "error",
};

export default function StatusChip({ status, label, ...props }) {
  const value = String(status || "unknown").toLowerCase();
  const text = value.replaceAll("_", " ");
  return <Chip size="small" variant="outlined" color={statusColors[value] || "default"}
    label={label || text.charAt(0).toUpperCase() + text.slice(1)} {...props} />;
}
