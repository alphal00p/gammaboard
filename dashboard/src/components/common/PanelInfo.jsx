import { useId, useState } from "react";
import { IconButton, Popover, Stack, Typography } from "@mui/material";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";

export default function PanelInfo({ title, notes }) {
  const [anchor, setAnchor] = useState(null);
  const id = useId();
  const paragraphs = [...new Set(notes.filter((note) => typeof note === "string" && note.trim()))];
  if (!paragraphs.length) return null;
  const label = `${title || "Observables"} information`;
  return <>
    <IconButton size="small" aria-label={label} aria-haspopup="dialog" aria-expanded={Boolean(anchor)}
      aria-controls={anchor ? id : undefined} onClick={(event) => setAnchor(event.currentTarget)}>
      <InfoOutlinedIcon fontSize="small" />
    </IconButton>
    <Popover id={id} open={Boolean(anchor)} anchorEl={anchor} onClose={() => setAnchor(null)}
      anchorOrigin={{ vertical: "bottom", horizontal: "right" }} transformOrigin={{ vertical: "top", horizontal: "right" }}
      slotProps={{ paper: { role: "dialog", "aria-label": label, sx: { p: 2, maxWidth: 440 } } }}>
      <Stack spacing={1.5}>{paragraphs.map((text) => <Typography key={text} variant="body2" sx={{ whiteSpace: "pre-line" }}>{text}</Typography>)}</Stack>
    </Popover>
  </>;
}
