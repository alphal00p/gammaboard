import { useState } from "react";
import {
  Alert,
  Button,
  Checkbox,
  FormControlLabel,
  MenuItem,
  Popover,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { prefixLabel } from "./strataUtils";

export const availablePdfPlots = (state, settings) => [
  ...(state.geometry.kind === "line" ? ["density"] : ["target", "sampler"]),
  settings.auxiliary,
  "densityHistogram",
  "mismatchHistogram",
];
export const PDF_PLOT_LABELS = {
  target: "Target",
  sampler: "Sampler",
  density: "Target / Sampler",
  mismatch: "Sampling mismatch",
  sign: "Projection sign",
  densityHistogram: "Target / Sampler distribution",
  mismatchHistogram: "Mismatch distribution",
};

export default function PdfExportMenu({ state, settings, scope, zoom, plot }) {
  const [anchor, setAnchor] = useState(null),
    [plots, setPlots] = useState([]),
    [strata, setStrata] = useState([]),
    [format, setFormat] = useState("pdf");
  const [extent, setExtent] = useState("current"),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(null);
  const open = (event) => {
    setPlots(plot ? [plot] : availablePdfPlots(state, settings));
    setStrata([scope]);
    setError(null);
    setAnchor(event.currentTarget);
  };
  const choices = [
    ["overall", "Overall"],
    ...(state.data.strata?.prefixes || []).map((p, k) => [
      String(k),
      `Stratum ${prefixLabel(p)}`,
    ]),
  ];
  return (
    <>
      <Button
        size="small"
        variant="outlined"
        aria-label={plot ? `Export ${PDF_PLOT_LABELS[plot]}` : "Export plots"}
        onClick={open}
      >
        Export ▾
      </Button>
      <Popover
        open={Boolean(anchor)}
        anchorEl={anchor}
        onClose={() => {
          if (!busy) setAnchor(null);
        }}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
      >
        <Stack spacing={1.5} sx={{ p: 2, width: 330, maxWidth: "90vw" }}>
          <Typography variant="subtitle2">
            Export {plot ? PDF_PLOT_LABELS[plot] : "plots"}
          </Typography>
          {!plot && (
            <TextField
              select
              label="Plots"
              value={plots}
              slotProps={{
                select: {
                  multiple: true,
                  renderValue: (values) => `${values.length} plots`,
                },
              }}
              onChange={(e) => setPlots(e.target.value)}
            >
              {availablePdfPlots(state, settings).map((key) => (
                <MenuItem key={key} value={key}>
                  <Checkbox checked={plots.includes(key)} />
                  {PDF_PLOT_LABELS[key]}
                </MenuItem>
              ))}
            </TextField>
          )}
          {choices.length > 1 && (
            <TextField
              select
              label="Strata"
              value={strata}
              slotProps={{
                select: {
                  multiple: true,
                  renderValue: (values) => `${values.length} selections`,
                },
              }}
              onChange={(e) => setStrata(e.target.value)}
            >
              {choices.map(([key, label]) => (
                <MenuItem key={key} value={key}>
                  <Checkbox checked={strata.includes(key)} />
                  {label}
                </MenuItem>
              ))}
            </TextField>
          )}
          <TextField
            select
            label="Format"
            value={format}
            onChange={(e) => setFormat(e.target.value)}
          >
            {["pdf", "svg", "png", "json", "csv"].map((f) => (
              <MenuItem key={f} value={f}>
                {f.toUpperCase()}
              </MenuItem>
            ))}
          </TextField>
          <FormControlLabel
            label="Full slice (otherwise current view)"
            control={
              <Checkbox
                checked={extent === "full"}
                onChange={(e) =>
                  setExtent(e.target.checked ? "full" : "current")
                }
              />
            }
          />
          {error && <Alert severity="error">{error}</Alert>}
          <Button
            variant="contained"
            disabled={busy || !plots.length || !strata.length}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                const { exportPdfDiagnostics } = await import(
                  "./pdfDiagnosticExport"
                );
                await exportPdfDiagnostics({
                  state,
                  settings,
                  scopes: strata,
                  plots,
                  format,
                  zoom: extent === "full" ? null : zoom,
                });
                setAnchor(null);
              } catch (e) {
                setError(e.message);
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? "Preparing export…" : "Download"}
          </Button>
        </Stack>
      </Popover>
    </>
  );
}
