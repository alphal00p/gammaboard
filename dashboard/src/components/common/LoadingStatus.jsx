import { useContext, useEffect, useId, useState } from "react";
import { Stack, CircularProgress, Tooltip, Typography } from "@mui/material";
import { TabLoadingContext, WorkspaceActivity } from "./workspaceContext";

// A short delay keeps routine polls from flashing while retaining useful feedback
// for slow requests. Existing content stays mounted underneath this status.
export default function LoadingStatus({ loading = false, label = "Updating…", compact = false }) {
  const [visible, setVisible] = useState(false);
  const active = useContext(WorkspaceActivity);
  const register = useContext(TabLoadingContext)?.register;
  const id = useId();
  useEffect(() => {
    if (!register || compact) return undefined;
    register(id, active && loading ? label : null);
    return () => register(id, null);
  }, [active, register, id, compact, loading, label]);
  useEffect(() => {
    if (!loading) { setVisible(false); return undefined; }
    const timer = setTimeout(() => setVisible(true), 150);
    return () => clearTimeout(timer);
  }, [loading]);
  if ((register && !compact) || !active || !loading || !visible) return null;
  if (compact) return <Tooltip title={label}><Stack direction="row" spacing={1} alignItems="center" role="status" aria-live="polite" aria-label={label} sx={{ minWidth: 0 }}>
    <CircularProgress size={14} aria-label={label} sx={{ flexShrink: 0 }} />
    <Typography variant="caption" color="text.secondary" sx={{ display: { xs: "none", md: "block" }, overflow: "hidden", whiteSpace: "nowrap", textOverflow: "ellipsis" }}>{label}</Typography>
  </Stack></Tooltip>;
  return <Stack direction="row" spacing={1} alignItems="center" role="status" aria-live="polite" sx={{ my: 1 }}>
    <CircularProgress size={14} aria-label={label} />
    <Typography variant="caption" color="text.secondary">{label}</Typography>
  </Stack>;
}
