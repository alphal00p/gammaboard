import { useEffect, useState } from "react";
import { Stack, CircularProgress, Typography } from "@mui/material";

// A short delay keeps routine polls from flashing while retaining useful feedback
// for slow requests. Existing content stays mounted underneath this status.
export default function LoadingStatus({ loading = false, label = "Updating…" }) {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!loading) { setVisible(false); return undefined; }
    const timer = setTimeout(() => setVisible(true), 150);
    return () => clearTimeout(timer);
  }, [loading]);
  if (!loading || !visible) return null;
  return <Stack direction="row" spacing={1} alignItems="center" role="status" aria-live="polite" sx={{ my: 1 }}>
    <CircularProgress size={14} aria-label={label} />
    <Typography variant="caption" color="text.secondary">{label}</Typography>
  </Stack>;
}
