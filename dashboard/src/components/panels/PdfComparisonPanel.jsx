import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Checkbox,
  FormControlLabel,
  MenuItem,
  Popover,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { PanelHeader, ControlBar } from "../common/PanelChrome";
import DisclosureField from "../common/DisclosureField";
import LazyChart from "./LazyChart";
import { DiagnosticSelect } from "./DiagnosticControls";
import { finite, prefixLabel } from "./strataUtils";
import {
  formatScientific,
  formatComparisonDecimal,
} from "../../utils/formatters";
import { FULL_ZOOM, readDataZoomRanges } from "./panelView";
import { pdfComparison } from "./pdfDiagnostics";
import {
  comparisonModels,
  colorGradient,
  curveOption,
  distributionOption,
  DENSITY_MAPS,
  mismatchValue,
  planeOption,
  rasterImage,
} from "./pdfDisplay";
import PdfExportMenu from "./PdfExportMenu";

const num = (v) => (finite(v) ? formatScientific(v, 3) : "Unavailable");
const gridStyle = {
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 320px), 1fr))",
  gap: 2,
};

function Colorbar({ model, onLimits, linked, onLink }) {
  const [anchor, setAnchor] = useState(null),
    [min, setMin] = useState(""),
    [max, setMax] = useState("");
  const [error, setError] = useState(false);
  if (model.kind === "sign")
    return (
      <Typography variant="caption">
        − orange · 0 white · + blue · unavailable grey
      </Typography>
    );
  const lower = model.log ? 10 ** model.limits.min : model.limits.min;
  const upper = model.log ? 10 ** model.limits.max : model.limits.max;
  const zero = -model.limits.min / (model.limits.max - model.limits.min);
  const open = (event) => {
    setMin(String(lower));
    setMax(String(upper));
    setError(false);
    setAnchor(event.currentTarget);
  };
  return (
    <>
      <Box
        component="button"
        aria-label={`${model.kind} color limits`}
        onClick={open}
        sx={{
          display: "block",
          width: "100%",
          border: 0,
          bgcolor: "transparent",
          color: "text.primary",
          cursor: "pointer",
          px: 2,
          py: 0.5,
        }}
      >
        <Box
          sx={{
            height: 12,
            background: colorGradient(model),
            borderRadius: 0.5,
          }}
        />
        <Box
          sx={{
            display: "flex",
            justifyContent: "space-between",
            position: "relative",
            typography: "caption",
          }}
        >
          <span>{num(lower)}</span>
          <span>{num(upper)}</span>
          {model.kind === "mismatch" && zero > 0.12 && zero < 0.88 && (
            <Box
              component="span"
              sx={{ position: "absolute", left: `${zero * 100}%` }}
            >
              0
            </Box>
          )}
        </Box>
      </Box>
      <Popover
        open={Boolean(anchor)}
        anchorEl={anchor}
        onClose={() => setAnchor(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "left" }}
      >
        <Stack spacing={1.5} sx={{ p: 2, width: 300 }}>
          <Typography variant="subtitle2">
            Color limits{model.log ? " (density units)" : ""}
          </Typography>
          <TextField
            label="Min"
            value={min}
            onChange={(e) => setMin(e.target.value)}
          />
          <TextField
            label="Max"
            value={max}
            onChange={(e) => setMax(e.target.value)}
          />
          {model.kind !== "mismatch" && (
            <FormControlLabel
              label="Link Target / Sampler"
              control={
                <Checkbox
                  checked={linked}
                  onChange={(e) => onLink(e.target.checked)}
                />
              }
            />
          )}
          {error && (
            <Alert severity="error">
              Enter finite Min &lt; Max{model.log ? ", both positive" : ""}.
            </Alert>
          )}
          <Stack direction="row" spacing={1}>
            <Button
              onClick={() => {
                onLimits(null);
                setAnchor(null);
              }}
            >
              Auto
            </Button>
            <Button
              onClick={() => {
                const a = Number(min),
                  b = Number(max);
                if (
                  !min.trim() ||
                  !max.trim() ||
                  !Number.isFinite(a) ||
                  !Number.isFinite(b) ||
                  a >= b ||
                  (model.log && a <= 0)
                ) {
                  setError(true);
                  return;
                }
                onLimits({ min: a, max: b });
                setAnchor(null);
              }}
            >
              Apply
            </Button>
          </Stack>
        </Stack>
      </Popover>
    </>
  );
}

const Figure = memo(function Figure({
  title,
  option,
  zoom,
  onNavigate,
  model,
  children,
  exportButton,
}) {
  const chart = useRef(null),
    tooltip = useRef(null),
    frame = useRef(null),
    pending = useRef(null);
  const callbacks = useRef({ onNavigate });
  useEffect(() => {
    callbacks.current = { onNavigate };
  }, [onNavigate]);
  const events = useMemo(
    () => ({
      datazoom: (event) => {
        pending.current = readDataZoomRanges(event);
        if (!frame.current)
          frame.current = requestAnimationFrame(() => {
            frame.current = null;
            callbacks.current.onNavigate?.(pending.current);
          });
      },
    }),
    [],
  );
  useEffect(() => () => cancelAnimationFrame(frame.current), []);
  const syncY = Boolean(model);
  const syncZoom = useCallback(() => {
    const instance = chart.current?.getEchartsInstance?.();
    if (!instance || !zoom) return;
    instance.dispatchAction(
      {
        type: "dataZoom",
        batch: [
          { dataZoomId: "x-inside", start: zoom.x.start, end: zoom.x.end },
          { dataZoomId: "x-slider", start: zoom.x.start, end: zoom.x.end },
          ...(syncY
            ? [
                {
                  dataZoomId: "y-inside",
                  start: zoom.y.start,
                  end: zoom.y.end,
                },
                {
                  dataZoomId: "y-slider",
                  start: zoom.y.start,
                  end: zoom.y.end,
                },
              ]
            : []),
        ],
      },
      { silent: true },
    );
  }, [zoom, syncY]);
  useEffect(syncZoom, [syncZoom, option]);
  const move = (event) => {
    if (!model || !tooltip.current) return;
    const instance = chart.current?.getEchartsInstance?.(),
      rect = event.currentTarget.getBoundingClientRect();
    const pixel = [event.clientX - rect.left, event.clientY - rect.top];
    if (!instance?.containPixel({ gridIndex: 0 }, pixel)) {
      tooltip.current.hidden = true;
      return;
    }
    const [ix, iy] = instance
      .convertFromPixel({ gridIndex: 0 }, pixel)
      .map(Math.round);
    const i = iy * model.c.axes.x.length + ix;
    if (
      ix < 0 ||
      ix >= model.c.axes.x.length ||
      iy < 0 ||
      iy >= model.c.axes.y.length
    ) {
      tooltip.current.hidden = true;
      return;
    }
    const v =
      model.kind === "mismatch"
        ? mismatchValue(model.c, i, model.metric)
        : model.values[i];
    tooltip.current.textContent = `u=${num(model.c.axes.x[ix])}, v=${num(model.c.axes.y[iy])}\n${model.kind}: ${v === Infinity ? "+∞" : v === -Infinity ? "−∞" : num(v)}\n${model.c.status[i]}`;
    tooltip.current.style.left = `${Math.min(pixel[0] + 10, Math.max(0, rect.width - 240))}px`;
    tooltip.current.style.top = `${Math.min(pixel[1] + 10, 280)}px`;
    tooltip.current.hidden = false;
  };
  return (
    <Box sx={{ minWidth: 0 }}>
      <PanelHeader title={title} sx={{ mb: 0 }}>
        {exportButton}
      </PanelHeader>
      <Box
        sx={{ position: "relative" }}
        onMouseMove={move}
        onMouseLeave={() => {
          if (tooltip.current) tooltip.current.hidden = true;
        }}
      >
        <LazyChart
          ref={chart}
          option={option}
          replaceMerge={["series"]}
          onChartReady={syncZoom}
          style={{ height: 340 }}
          onEvents={onNavigate ? events : undefined}
        />
        <Box
          ref={tooltip}
          hidden
          sx={{
            position: "absolute",
            pointerEvents: "none",
            zIndex: 2,
            bgcolor: "background.paper",
            boxShadow: 2,
            p: 1,
            whiteSpace: "pre-line",
            typography: "caption",
          }}
        />
      </Box>
      {children}
    </Box>
  );
});

const PlaneFigure = memo(function PlaneFigure({ model, zoom, ...props }) {
  const raster = useMemo(() => rasterImage(model), [model]);
  const option = useMemo(() => planeOption(model, {}, raster), [model, raster]);
  return <Figure {...props} model={model} zoom={zoom} option={option} />;
});

function PdfComparisonPanel({ title, state, value = {}, onValueChange }) {
  const { data, geometry } = state;
  const prefixes = data.strata?.prefixes || [];
  const scope =
    prefixes.length &&
    value?.scope != null &&
    Number.isInteger(Number(value?.scope)) &&
    Number(value.scope) >= 0 &&
    Number(value.scope) < prefixes.length
      ? String(value.scope)
      : "overall";
  const reference =
    state.data.reference ||
    (finite(state.data.global_abs_integrand_norm) &&
    state.data.global_abs_integrand_norm > 0
      ? { value: state.data.global_abs_integrand_norm, source: "legacy_source" }
      : null);
  const projection = state.data.projection;
  const hasSign = projection
    ? !projection.nonnegative
    : state.data.integrand_values.some((v) => finite(v) && v < 0);
  const mode =
    value?.normalization === "reference" && reference
      ? "reference"
      : value?.normalization == null && reference
        ? "reference"
        : "slice";
  const metric = value?.metric === "relative" ? "relative" : "log_ratio";
  const auxiliary =
    value?.auxiliary === "sign" && hasSign ? "sign" : "mismatch";
  const log = Boolean(value?.log),
    palette = value?.palette || "Viridis",
    linked = value?.linked !== false;
  const limits = value?.limits;
  const settings = useMemo(
    () => ({
      normalization: mode,
      metric,
      auxiliary,
      log,
      palette,
      linked,
      limits,
      histogramLog: Boolean(value?.histogramLog),
      histogramWeighted: value?.histogramWeighted !== false,
    }),
    [
      mode,
      metric,
      auxiliary,
      log,
      palette,
      linked,
      limits,
      value?.histogramLog,
      value?.histogramWeighted,
    ],
  );
  // Navigation is deliberately absent: changing the view never recalculates a grid or histogram.
  const comparison = useMemo(
    () => pdfComparison({ data, geometry }, scope, mode),
    [data, geometry, scope, mode],
  );
  const models = useMemo(
    () => comparisonModels({ data, geometry }, scope, settings, comparison),
    [data, geometry, scope, settings, comparison],
  );
  const line = state.geometry.kind === "line";
  const curves = useMemo(
    () => ({
      density: line ? curveOption(models, settings) : null,
      auxiliary: line ? curveOption(models, settings, true) : null,
    }),
    [models, settings, line],
  );
  const histogramLog = Boolean(value?.histogramLog);
  const histogramWeighted = value?.histogramWeighted !== false;
  const densityHistogram = useMemo(
    () =>
      distributionOption(
        { c: comparison },
        { histogramLog, histogramWeighted },
      ),
    [comparison, histogramLog, histogramWeighted],
  );
  const mismatchHistogram = useMemo(
    () =>
      distributionOption(
        {
          c: comparison,
          mismatch: {
            values:
              metric === "relative" ? comparison.relative : comparison.logRatio,
          },
        },
        { metric, histogramWeighted },
        true,
      ),
    [comparison, metric, histogramWeighted],
  );
  const options = { ...curves, densityHistogram, mismatchHistogram };
  const current = useRef(value);
  useEffect(() => {
    current.current = value;
  }, [value]);
  const update = useCallback(
    (patch) =>
      onValueChange?.(state.panel_id, { ...current.current, ...patch }, false),
    [onValueChange, state.panel_id],
  );
  const navigate = useCallback(
    (ranges) => {
      if (ranges.x || (!line && ranges.y))
        update({
          ...(ranges.x ? { zoom: ranges.x } : {}),
          ...(!line && ranges.y ? { yZoom: ranges.y } : {}),
        });
    },
    [update, line],
  );
  const zoom = useMemo(
    () => ({ x: value?.zoom || FULL_ZOOM, y: value?.yZoom || FULL_ZOOM }),
    [value?.zoom, value?.yZoom],
  );
  const change = (patch) => update({ ...patch, limits: {} });
  const setLimits = (role, next) =>
    update({
      limits: {
        ...limits,
        [linked && role !== "mismatch" ? "density" : role]: next,
      },
    });
  const bar = (model) => (
    <Colorbar
      model={model}
      linked={linked}
      onLink={(next) => change({ linked: next })}
      onLimits={(next) => setLimits(model.kind, next)}
    />
  );
  const exports = (plot) => (
    <PdfExportMenu
      state={state}
      settings={settings}
      scope={scope}
      zoom={zoom}
      plot={plot}
    />
  );
  const counts = models.c.counts;
  return (
    <Card>
      <CardContent>
        <Stack spacing={2}>
          <PanelHeader title={title} sx={{ mb: 0 }}>
            {exports(null)}
          </PanelHeader>
          <ControlBar>
            <TextField
              select
              label="Normalization"
              value={mode}
              sx={{ minWidth: 230 }}
              onChange={(e) => change({ normalization: e.target.value })}
            >
              <MenuItem value="reference" disabled={!reference}>
                Source integral reference
              </MenuItem>
              <MenuItem value="slice">Slice</MenuItem>
            </TextField>
            {prefixes.length > 0 && (
              <DiagnosticSelect
                label="Stratum"
                value={scope}
                onChange={(next) => change({ scope: next })}
                options={[
                  { value: "overall", label: "Overall" },
                  ...prefixes.map((p, k) => ({
                    value: String(k),
                    label: `Stratum ${prefixLabel(p)}`,
                  })),
                ]}
              />
            )}
            <DisclosureField
              inline
              label="Normalization details"
              ariaLabel="Normalization details"
              value={
                reference && mode === "reference"
                  ? `Reference ${num(reference.value)} · SE ${finite(reference.error) ? `${formatComparisonDecimal((100 * reference.error) / reference.value)}%` : "unavailable"}`
                  : "Mean-one slice comparison"
              }
            >
              <Typography variant="body2">
                Target:{" "}
                {state.data.target_label || "Absolute training projection"}.
                Source reference compares absolute density levels; Slice
                compares shapes and removes overall normalization differences.
                These are slices, not marginals. Zooming does not change
                normalization.
              </Typography>
              {reference && (
                <Typography variant="body2">
                  Reference source: {reference.source.replaceAll("_", " ")}
                  {reference.task_name ? ` · ${reference.task_name}` : ""}
                  {reference.task_id ? ` (task ${reference.task_id})` : ""}.
                  Value: {String(reference.value)}; standard error:{" "}
                  {reference.error == null
                    ? "unavailable"
                    : String(reference.error)}
                  {reference.sample_count != null
                    ? `; samples: ${reference.sample_count}`
                    : ""}
                  .
                </Typography>
              )}
              {prefixes.length > 0 && (
                <Typography variant="body2">
                  Source mode retains joint stratum PDFs, including their
                  allocation probabilities. Mismatch therefore includes
                  allocation and within-stratum shape. Slice mode removes the
                  overall scale difference.
                </Typography>
              )}
            </DisclosureField>
          </ControlBar>
          <ControlBar>
            {!line && (
              <TextField
                select
                label="Colormap"
                value={palette}
                onChange={(e) => update({ palette: e.target.value })}
              >
                {Object.keys(DENSITY_MAPS).map((name) => (
                  <MenuItem key={name} value={name}>
                    {name}
                  </MenuItem>
                ))}
              </TextField>
            )}
            <TextField
              select
              label="Scale"
              sx={{ minWidth: 160 }}
              value={log ? "log" : "linear"}
              onChange={(e) => change({ log: e.target.value === "log" })}
            >
              <MenuItem value="linear">Linear</MenuItem>
              <MenuItem value="log">Logarithmic</MenuItem>
            </TextField>
            <TextField
              select
              label="Auxiliary"
              value={auxiliary}
              onChange={(e) => update({ auxiliary: e.target.value })}
            >
              <MenuItem value="mismatch">Sampling mismatch</MenuItem>
              {hasSign && <MenuItem value="sign">Projection sign</MenuItem>}
            </TextField>
            <TextField
              select
              label="Mismatch metric"
              value={metric}
              onChange={(e) => change({ metric: e.target.value })}
            >
              <MenuItem value="log_ratio">log₁₀(PDF / target)</MenuItem>
              <MenuItem value="relative">PDF / target − 1</MenuItem>
            </TextField>
            <Button
              onClick={() => update({ zoom: FULL_ZOOM, yZoom: FULL_ZOOM })}
            >
              Reset view
            </Button>
          </ControlBar>
          {(!models.c.target.some(finite) || !models.c.pdf.some(finite)) && (
            <Alert severity="info">
              Normalization unavailable for one or both fields. Slice
              normalization requires complete data and a positive mean.
            </Alert>
          )}
          <Box sx={gridStyle}>
            {line ? (
              <>
                <Figure
                  title="Target / Sampler"
                  option={options.density}
                  zoom={zoom}
                  onNavigate={navigate}
                  exportButton={exports("density")}
                />
                <Figure
                  title={
                    auxiliary === "sign"
                      ? "Projection sign"
                      : "Sampling mismatch"
                  }
                  option={options.auxiliary}
                  zoom={zoom}
                  onNavigate={navigate}
                  exportButton={exports(auxiliary)}
                />
              </>
            ) : (
              <>
                <PlaneFigure
                  title="Target"
                  model={models.target}
                  zoom={zoom}
                  onNavigate={navigate}
                  exportButton={exports("target")}
                >
                  {bar(models.target)}
                </PlaneFigure>
                <PlaneFigure
                  title="Sampler"
                  model={models.sampler}
                  zoom={zoom}
                  onNavigate={navigate}
                  exportButton={exports("sampler")}
                >
                  {bar(models.sampler)}
                </PlaneFigure>
                <PlaneFigure
                  title={
                    auxiliary === "sign"
                      ? "Projection sign"
                      : "Sampling mismatch"
                  }
                  model={models[auxiliary]}
                  zoom={zoom}
                  onNavigate={navigate}
                  exportButton={exports(auxiliary)}
                >
                  {bar(models[auxiliary])}
                </PlaneFigure>
              </>
            )}
          </Box>
          <DisclosureField
            section
            label="Coverage"
            value={`Zero PDF: ${counts.uncovered} · zero target: ${counts.leakage} · both zero: ${counts.both_zero} · unavailable: ${counts.missing}`}
          >
            <Typography variant="body2">
              Positive target with zero PDF is red (−1 relative mismatch, −∞ log
              ratio). Zero target with positive PDF is blue (+∞). Both zero
              gives an undefined ratio; missing values are grey. On logarithmic
              density plots, zero is light grey. Finite histogram densities
              exclude nonfinite values; logarithmic value histograms also
              exclude zeros.
            </Typography>
            {scope !== "overall" && (
              <Typography variant="body2">
                Requested stratum probability: {num(models.c.probability)}. PDF
                leakage on this slice: {num(models.c.leakage)}. Target divisor:{" "}
                {num(models.c.targetDivisor)}. PDF divisor:{" "}
                {num(models.c.pdfDivisor)}.
              </Typography>
            )}
          </DisclosureField>
          <ControlBar>
            <TextField
              select
              label="Scale"
              sx={{ minWidth: 160 }}
              value={settings.histogramLog ? "log" : "linear"}
              onChange={(e) =>
                update({ histogramLog: e.target.value === "log" })
              }
            >
              <MenuItem value="linear">Linear</MenuItem>
              <MenuItem value="log">Logarithmic</MenuItem>
            </TextField>
            <FormControlLabel
              label="Weight by sampler density"
              control={
                <Switch
                  checked={histogramWeighted}
                  onChange={(e) =>
                    update({ histogramWeighted: e.target.checked })
                  }
                />
              }
            />
            <DisclosureField
              inline
              label="Histogram definitions"
              value="Distributions across grid points"
            >
              <Typography variant="body2">
                With weighting enabled, each grid point contributes its sampler
                density to both histograms, using the selected stratum's joint
                PDF. Disabling weighting gives every grid point equal weight.
                Bin densities are divided by the total included weight and
                actual bin width. Zero-density regions have zero weight; use
                unweighted histograms and Coverage to inspect them. These are
                deterministic grid diagnostics with no Monte Carlo error bars.
                Spatial zoom does not change their population.
              </Typography>
              <Typography variant="body2">
                Target zeros: {models.c.target.filter((v) => v === 0).length};
                Sampler zeros: {models.c.pdf.filter((v) => v === 0).length}.
                Nonfinite mismatch values:{" "}
                {models.mismatch.values.filter((v) => !finite(v)).length}.
              </Typography>
            </DisclosureField>
          </ControlBar>
          {histogramWeighted &&
            !comparison.joint.some((v) => finite(v) && v > 0) && (
              <Alert severity="info">
                No positive sampler density is available for weighted
                histograms. Disable weighting to inspect the grid values.
              </Alert>
            )}
          <Box sx={gridStyle}>
            <Figure
              title="Target / Sampler distribution"
              option={options.densityHistogram}
              exportButton={exports("densityHistogram")}
            />
            <Figure
              title="Mismatch distribution"
              option={options.mismatchHistogram}
              exportButton={exports("mismatchHistogram")}
            />
          </Box>
        </Stack>
      </CardContent>
    </Card>
  );
}

export default memo(PdfComparisonPanel);
