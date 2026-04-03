import { useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  Button,
  FormControl,
  LinearProgress,
  MenuItem,
  Stack,
  Table as MuiTable,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Select,
  Typography,
} from "@mui/material";
import Plot from "react-plotly.js";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  ErrorBar,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Scatter,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { formatCompactNumber, formatDateTime, formatScientific } from "../../utils/formatters";
import { asArray } from "../../utils/collections";

const buildRenderablePanels = (panelSpecs, panelStates, panelValues) => {
  const stateMap = new Map(asArray(panelStates).map((panel) => [panel.panel_id, panel]));
  const renderablePanels = asArray(panelSpecs).map((spec) => ({
    descriptor: spec,
    state: stateMap.get(spec.panel_id) || null,
    value: panelValues?.[spec.panel_id],
  }));
  const bundlePanel = renderablePanels.find(({ descriptor }) => descriptor?.panel_id === "gammaloop_histogram_bundle");
  const payload = bundlePanel?.state?.payload;
  const histograms = payload?.histograms;
  if (bundlePanel && histograms && typeof histograms === "object" && !Array.isArray(histograms)) {
    const selectedName =
      bundlePanel.value ??
      payload?.primary_histogram_name ??
      Object.keys(histograms).find((key) => key && typeof histograms[key] === "object") ??
      null;
    const selectedHistogram =
      (selectedName && histograms[selectedName]) ||
      (payload?.primary_histogram_name && histograms[payload.primary_histogram_name]) ||
      Object.values(histograms).find((entry) => entry && typeof entry === "object") ||
      null;
    if (selectedHistogram) {
      renderablePanels.push({
        descriptor: {
          panel_id: "gammaloop_selected_histogram",
          label: "Selected Histogram",
          kind: "histogram",
          history: "none",
          width: "full",
        },
        state: {
          panel_id: "gammaloop_selected_histogram",
          name: selectedName,
          title: selectedHistogram.title,
          type_description: selectedHistogram.type_description,
          phase: selectedHistogram.phase,
          value_transform: selectedHistogram.value_transform,
          sample_count: selectedHistogram.sample_count,
          x_min: selectedHistogram.x_min,
          x_max: selectedHistogram.x_max,
          log_x_axis: selectedHistogram.log_x_axis,
          log_y_axis: selectedHistogram.log_y_axis,
          bins: asArray(selectedHistogram.bins),
        },
        value: null,
      });
    }
  }
  return renderablePanels;
};

const fitDomain = (values) => {
  const finiteValues = values.filter((value) => Number.isFinite(value));
  if (finiteValues.length === 0) return ["auto", "auto"];
  const min = Math.min(...finiteValues);
  const max = Math.max(...finiteValues);
  if (min === max) {
    const padding = Math.abs(min) > 0 ? Math.abs(min) * 0.1 : 1;
    return [min - padding, max + padding];
  }
  const padding = (max - min) * 0.08;
  return [min - padding, max + padding];
};

const fitXDomain = (values) => {
  const finiteValues = values.filter((value) => Number.isFinite(value));
  if (finiteValues.length === 0) return ["auto", "auto"];
  const min = Math.min(...finiteValues);
  const max = Math.max(...finiteValues);
  if (min === max) {
    const padding = Math.abs(min) > 0 ? Math.abs(min) * 0.1 : 1;
    return [min - padding, max + padding];
  }
  return [min, max];
};

const fitHistogramXDomain = (bins) => {
  const edges = bins.flatMap((bin) => [bin.start, bin.stop]).filter((value) => Number.isFinite(value));
  return fitXDomain(edges);
};

const buildMultiSeriesData = (seriesList) => {
  const rows = new Map();
  for (const series of asArray(seriesList)) {
    for (const point of asArray(series.points)) {
      const row = rows.get(point.x) || { x: point.x };
      row[series.id] = point.y;
      rows.set(point.x, row);
    }
  }
  return Array.from(rows.values()).sort((a, b) => a.x - b.x);
};

const lineColors = ["#005f73", "#bb3e03", "#0a9396", "#ae2012", "#ca6702"];

const bandColor = "rgba(10, 147, 150, 0.18)";
const invalidImageColor = [255, 0, 255];

const isIsoDateTime = (value) => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value);

const renderStructuredValue = (value) => {
  if (value == null) return "none";
  if (typeof value === "number") return formatCompactNumber(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "string") {
    if (!value.trim()) return "none";
    if (isIsoDateTime(value)) return formatDateTime(value, value);
    return value;
  }
  return JSON.stringify(value);
};

const scalarHeatmapColorscale = [
  [0, "rgb(0,0,255)"],
  [0.5, "rgb(128,200,128)"],
  [1, "rgb(255,0,0)"],
];

const ComplexImageTooltip = ({ hover }) => {
  if (!hover) return null;
  return (
    <Card
      variant="outlined"
      sx={{
        position: "absolute",
        left: hover.left,
        top: hover.top,
        transform: "translate(12px, 12px)",
        pointerEvents: "none",
        zIndex: 2,
        minWidth: 180,
      }}
    >
      <CardContent sx={{ py: 1, px: 1.25, "&:last-child": { pb: 1 } }}>
        <Typography variant="caption" sx={{ display: "block", color: "text.secondary", mb: 0.5 }}>
          x={formatScientific(hover.x, 4)} y={formatScientific(hover.y, 4)}
        </Typography>
        <Typography variant="body2">re: {formatScientific(hover.re, 6)}</Typography>
        <Typography variant="body2">im: {formatScientific(hover.im, 6)}</Typography>
        <Typography variant="body2">|z|: {formatScientific(hover.magnitude, 6)}</Typography>
        <Typography variant="body2">phase: {formatScientific(hover.phase, 5)} rad</Typography>
      </CardContent>
    </Card>
  );
};

const panelColumnSpan = (descriptor) => {
  switch (descriptor?.width) {
    case "compact":
      return { xs: "1 / -1", md: "span 4" };
    case "full":
      return { xs: "1 / -1", md: "1 / -1" };
    case "half":
      return { xs: "1 / -1", md: "span 6" };
    default:
      switch (descriptor?.kind) {
        case "scalar_timeseries":
        case "multi_timeseries":
        case "tick_breakdown":
        case "image2d":
        case "table":
        case "histogram":
          return { xs: "1 / -1", md: "1 / -1" };
        case "progress":
        case "key_value":
        case "text":
        case "select":
        default:
          return { xs: "1 / -1", md: "span 6" };
      }
  }
};

const chartMargin = { top: 8, right: 16, left: 8, bottom: 8 };
const gridColor = "rgba(148,163,184,0.18)";
const axisTickStyle = { fontSize: 12, fill: "#64748b" };

const formatAxisValue = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? formatScientific(numeric, 3) : "";
};

const SimpleChartTooltip = ({ active, payload, label, labelPrefix = "x" }) => {
  if (!active || !payload?.length) return null;
  return (
    <Card variant="outlined" sx={{ pointerEvents: "none" }}>
      <CardContent sx={{ py: 1, px: 1.25, "&:last-child": { pb: 1 } }}>
        <Typography variant="caption" sx={{ display: "block", color: "text.secondary", mb: 0.5 }}>
          {labelPrefix}={formatScientific(Number(label), 5)}
        </Typography>
        {payload
          .filter((entry) => entry?.value != null && Number.isFinite(Number(entry.value)) && !entry?.hide)
          .map((entry) => (
            <Typography key={entry.dataKey} variant="body2">
              {entry.name || entry.dataKey}: {formatScientific(Number(entry.value), 6)}
            </Typography>
          ))}
      </CardContent>
    </Card>
  );
};

const HistogramTooltip = ({ active, payload }) => {
  if (!active || !payload?.length) return null;
  const row = payload.find((entry) => entry?.payload)?.payload;
  if (!row) return null;
  return (
    <Card variant="outlined" sx={{ pointerEvents: "none" }}>
      <CardContent sx={{ py: 1, px: 1.25, "&:last-child": { pb: 1 } }}>
        <Typography variant="caption" sx={{ display: "block", color: "text.secondary", mb: 0.5 }}>
          {row.rangeLabel}
        </Typography>
        {Number.isFinite(row.value) ? (
          <Typography variant="body2">value: {formatScientific(row.value, 6)}</Typography>
        ) : null}
        {Number.isFinite(row.error) ? (
          <Typography variant="body2">error: {formatScientific(row.error, 6)}</Typography>
        ) : null}
        {Number.isFinite(row.relative_error) ? (
          <Typography variant="body2">rel err: {formatScientific(row.relative_error, 6)}</Typography>
        ) : null}
      </CardContent>
    </Card>
  );
};

const ScalarTimeseriesPanel = ({ title, state }) => {
  const points = asArray(state?.points)
    .slice()
    .sort((a, b) => a.x - b.x);
  if (points.length === 0) return null;
  const domain = fitDomain(points.flatMap((point) => [point.y, point.y_min, point.y_max]));
  const xDomain = fitXDomain(points.map((point) => point.x));
  const hasBand = points.some((point) => Number.isFinite(point.y_min) && Number.isFinite(point.y_max));
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Box sx={{ width: "100%", height: 280 }}>
          <ResponsiveContainer width="100%" height="100%">
            <ComposedChart data={points} margin={chartMargin}>
              <CartesianGrid stroke={gridColor} vertical={false} />
              <XAxis dataKey="x" type="number" domain={xDomain} tickFormatter={formatAxisValue} tick={axisTickStyle} />
              <YAxis domain={domain} tickFormatter={formatAxisValue} tick={axisTickStyle} width={72} />
              <Tooltip content={<SimpleChartTooltip />} />
              {hasBand ? (
                <>
                  <Area dataKey="y_min" stroke="none" fillOpacity={0} isAnimationActive={false} />
                  <Area
                    dataKey="y_max"
                    stroke="none"
                    fill={bandColor}
                    fillOpacity={1}
                    baseLine={(x) => x?.y_min}
                    isAnimationActive={false}
                  />
                </>
              ) : null}
              <Line
                type="monotone"
                dataKey="y"
                stroke="#005f73"
                strokeWidth={1.8}
                dot={false}
                isAnimationActive={false}
              />
            </ComposedChart>
          </ResponsiveContainer>
        </Box>
      </CardContent>
    </Card>
  );
};

const MultiTimeseriesPanel = ({ title, state }) => {
  const series = asArray(state?.series);
  const data = buildMultiSeriesData(series);
  if (data.length === 0) return null;
  const domain = fitDomain(
    data.flatMap((row) =>
      Object.entries(row)
        .filter(([key]) => key !== "x")
        .map(([, value]) => value),
    ),
  );
  const xDomain = fitXDomain(data.map((row) => row.x));
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Box sx={{ width: "100%", height: 280 }}>
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={data} margin={chartMargin}>
              <CartesianGrid stroke={gridColor} vertical={false} />
              <XAxis dataKey="x" type="number" domain={xDomain} tickFormatter={formatAxisValue} tick={axisTickStyle} />
              <YAxis domain={domain} tickFormatter={formatAxisValue} tick={axisTickStyle} width={72} />
              <Tooltip content={<SimpleChartTooltip />} />
              {series.map((item, index) => (
                <Line
                  key={item.id}
                  type="monotone"
                  dataKey={item.id}
                  name={item.label}
                  stroke={lineColors[index % lineColors.length]}
                  strokeWidth={1.8}
                  dot={false}
                  connectNulls={false}
                  isAnimationActive={false}
                />
              ))}
            </LineChart>
          </ResponsiveContainer>
        </Box>
      </CardContent>
    </Card>
  );
};

const buildHistogramData = (bins) =>
  asArray(bins)
    .map((bin) => {
      const start = Number(bin?.start);
      const stop = Number(bin?.stop);
      const value = Number(bin?.value);
      const error = Number(bin?.error);
      const x = Number.isFinite(start) && Number.isFinite(stop) ? (start + stop) / 2 : Number.NaN;
      return {
        ...bin,
        start,
        stop,
        x,
        value,
        error: Number.isFinite(error) ? error : 0,
        rangeLabel:
          Number.isFinite(start) && Number.isFinite(stop)
            ? `${formatScientific(start, 4)} → ${formatScientific(stop, 4)}`
            : "n/a",
      };
    })
    .filter((bin) => Number.isFinite(bin.value) && Number.isFinite(bin.x));

const buildHistogramStepData = (bins) => {
  const orderedBins = asArray(bins)
    .slice()
    .sort((left, right) => left.start - right.start);
  const points = [];
  for (const [index, bin] of orderedBins.entries()) {
    points.push({
      x: bin.start,
      y: bin.value,
      error: bin.error,
      rangeLabel: `${formatScientific(bin.start, 4)} → ${formatScientific(bin.stop, 4)}`,
    });
    points.push({
      x: bin.stop,
      y: bin.value,
      error: bin.error,
      rangeLabel: `${formatScientific(bin.start, 4)} → ${formatScientific(bin.stop, 4)}`,
    });
    const nextBin = orderedBins[index + 1];
    if (nextBin && nextBin.start !== bin.stop) {
      points.push({
        x: bin.stop,
        y: nextBin.value,
        error: nextBin.error,
        rangeLabel: `${formatScientific(nextBin.start, 4)} → ${formatScientific(nextBin.stop, 4)}`,
      });
    }
  }
  return points.filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));
};

const buildHistogramRenderData = (bins, scale) => {
  const stepData = buildHistogramStepData(bins);
  if (scale !== "log") return stepData;
  return stepData.map((point) => ({
    ...point,
    y: Math.max(point.y, Number.EPSILON),
    error: Number.isFinite(point.error) ? point.error : 0,
  }));
};

const buildHistogramErrorBarData = (bins, scale) =>
  asArray(bins)
    .map((bin) => {
      const x = Number(bin?.x);
      const value = Number(bin?.value);
      const error = Number(bin?.error);
      if (!Number.isFinite(x) || !Number.isFinite(value)) return null;
      return {
        x,
        y: scale === "log" ? Math.max(value, Number.EPSILON) : value,
        error: Number.isFinite(error) ? error : 0,
        rangeLabel: bin?.rangeLabel || "n/a",
      };
    })
    .filter(Boolean);

const buildRelativeErrorStepData = (bins) =>
  buildHistogramStepData(bins)
    .map((point) => {
      const value = Number(point?.y);
      const error = Number(point?.error);
      if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) {
        return {
          ...point,
          relative_error: null,
          positive_relative_error: null,
          negative_relative_error: null,
        };
      }
      const relativeError = Math.abs(error / value);
      return {
        ...point,
        relative_error: relativeError,
        positive_relative_error: relativeError,
        negative_relative_error: -relativeError,
      };
    })
    .filter((point) => Number.isFinite(point.x));

const toExponential8 = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric.toExponential(8) : "0.00000000e+00";
};

const downloadTextFile = (filename, contents, mimeType = "text/plain;charset=utf-8") => {
  const blob = new Blob([contents], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = "noreferrer";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
};

const buildHistogramHwUBlock = (name, histogram) => {
  const bins = asArray(histogram?.bins);
  const title = histogram?.title ?? name ?? "histogram";
  const xAxisMode = histogram?.log_x_axis ? "LOG" : "LIN";
  const yAxisMode = histogram?.log_y_axis ? "LOG" : "LIN";
  const typeDescription = histogram?.type_description ?? "HwU";
  const xMin = Number.isFinite(Number(histogram?.x_min)) ? Number(histogram.x_min) : Number(bins[0]?.start);
  const xMax = Number.isFinite(Number(histogram?.x_max))
    ? Number(histogram.x_max)
    : Number(bins[bins.length - 1]?.stop);
  return [
    "##& xmin & xmax & central value & dy &",
    "",
    `<histogram> ${bins.length} "${title} |X_AXIS@${xAxisMode} |Y_AXIS@${yAxisMode} |TYPE@${typeDescription}"`,
    ...bins.map((bin) =>
      [
        `  ${toExponential8(Number(bin?.start) ?? xMin)}`,
        `${toExponential8(Number(bin?.stop) ?? xMax)}`,
        `${toExponential8(Number(bin?.value) ?? 0)}`,
        `${toExponential8(Number(bin?.error) ?? 0)}`,
      ].join("   "),
    ),
    "<\\histogram>",
    "",
  ].join("\n");
};

const buildHistogramBundleJson = (payload) => ({
  primary_histogram_name: payload?.primary_histogram_name ?? null,
  histograms: payload?.histograms ?? {},
});

const buildHistogramBundleHwU = (payload) => {
  const histograms = payload?.histograms && typeof payload.histograms === "object" ? payload.histograms : {};
  return Object.entries(histograms)
    .map(([name, histogram]) => buildHistogramHwUBlock(name, histogram))
    .join("\n");
};

const buildHistogramYDomain = (bins, scale) => {
  const values = asArray(bins)
    .flatMap((bin) => [
      Number(bin?.y ?? bin?.value) - Number(bin?.error || 0),
      Number(bin?.y ?? bin?.value) + Number(bin?.error || 0),
      Number(bin?.y ?? bin?.value),
    ])
    .filter((value) => Number.isFinite(value));
  if (values.length === 0) return ["auto", "auto"];
  if (scale === "log") {
    const positive = values.filter((value) => value > 0);
    if (positive.length === 0) return ["auto", "auto"];
    const min = Math.min(...positive);
    const max = Math.max(...positive);
    return [Math.max(min / 2, Number.EPSILON), max * 1.08];
  }
  return fitDomain(values);
};

const buildRelativeErrorYDomain = (points) => {
  const maxRelativeError = Math.max(
    0,
    ...asArray(points)
      .map((point) => Number(point?.relative_error))
      .filter((value) => Number.isFinite(value)),
  );
  if (maxRelativeError <= 0) return [-1, 1];
  const padded = maxRelativeError * 1.08;
  return [-padded, padded];
};

const buildImageRows = (values, width, height, invalidIndices = null) => {
  const rows = [];
  for (let row = 0; row < height; row += 1) {
    const current = [];
    for (let col = 0; col < width; col += 1) {
      const index = row * width + col;
      const value = values[index];
      current.push(invalidIndices?.has(index) || !Number.isFinite(value) ? null : value);
    }
    rows.push(current);
  }
  return rows;
};

const buildCellCenters = (range, count) => {
  const [min, max] = asArray(range);
  if (!Number.isFinite(min) || !Number.isFinite(max) || count <= 0) {
    return Array.from({ length: count }, (_, index) => index);
  }
  const step = (max - min) / count;
  return Array.from({ length: count }, (_, index) => min + step * (index + 0.5));
};

const buildScalarHeatmapScale = (values, normalizationMode) => {
  const finite = values.filter((value) => Number.isFinite(value));
  if (finite.length === 0) {
    return { zmin: 0, zmax: 1 };
  }
  if (normalizationMode === "symmetric") {
    const maxAbs = Math.max(...finite.map((value) => Math.abs(value)), 1e-12);
    return { zmin: -maxAbs, zmax: maxAbs };
  }
  const zmin = Math.min(...finite);
  const zmax = Math.max(...finite);
  if (zmin === zmax) {
    const padding = Math.abs(zmin) > 0 ? Math.abs(zmin) * 0.1 : 1;
    return { zmin: zmin - padding, zmax: zmax + padding };
  }
  return { zmin, zmax };
};

const buildInvalidCellOverlay = (invalidIndices, width, xCenters, yCenters) => {
  const points = Array.from(invalidIndices)
    .map((index) => {
      const row = Math.floor(index / width);
      const col = index % width;
      return {
        x: xCenters[col],
        y: yCenters[row],
      };
    })
    .filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));

  if (points.length === 0) return null;

  return {
    type: "scatter",
    mode: "markers",
    x: points.map((point) => point.x),
    y: points.map((point) => point.y),
    marker: {
      color: "#ff00ff",
      symbol: "square",
      size: 8,
      line: { width: 0 },
    },
    hovertemplate: "invalid value<extra></extra>",
    showlegend: false,
  };
};

const ScalarImageHeatmapPanel = ({
  title,
  width,
  height,
  values,
  invalidIndices,
  normalizationMode,
  xRange,
  yRange,
}) => {
  const z = useMemo(
    () => buildImageRows(values, width, height, invalidIndices),
    [height, invalidIndices, values, width],
  );
  const xCenters = useMemo(() => buildCellCenters(xRange, width), [width, xRange]);
  const yCenters = useMemo(() => buildCellCenters(yRange, height), [height, yRange]);
  const { zmin, zmax } = useMemo(() => buildScalarHeatmapScale(values, normalizationMode), [normalizationMode, values]);
  const invalidOverlay = useMemo(
    () => buildInvalidCellOverlay(invalidIndices, width, xCenters, yCenters),
    [invalidIndices, width, xCenters, yCenters],
  );

  const data = [
    {
      type: "heatmap",
      z,
      x: xCenters,
      y: yCenters,
      zmin,
      zmax,
      colorscale: scalarHeatmapColorscale,
      colorbar: {
        title: { text: "Value", side: "right" },
        thickness: 14,
        len: 0.88,
      },
      hovertemplate: "value: %{z:.6g}<extra></extra>",
      showscale: true,
    },
    ...(invalidOverlay ? [invalidOverlay] : []),
  ];

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Box sx={{ width: "100%", minHeight: 360 }}>
          <Plot
            data={data}
            layout={{
              autosize: true,
              margin: { l: 12, r: 56, t: 8, b: 8 },
              paper_bgcolor: "rgba(0,0,0,0)",
              plot_bgcolor: "rgba(0,0,0,0)",
              xaxis: {
                visible: false,
                showgrid: false,
                zeroline: false,
                constrain: "domain",
              },
              yaxis: {
                visible: false,
                showgrid: false,
                zeroline: false,
                scaleanchor: "x",
                autorange: "reversed",
              },
            }}
            config={{
              displayModeBar: false,
              responsive: true,
            }}
            style={{ width: "100%", height: "360px" }}
            useResizeHandler
          />
        </Box>
      </CardContent>
    </Card>
  );
};

const HistogramPanel = ({ title, state }) => {
  const [scale, setScale] = useState("linear");
  const bins = useMemo(() => buildHistogramData(state?.bins), [state?.bins]);
  const stepData = useMemo(() => buildHistogramRenderData(state?.bins, scale), [scale, state?.bins]);
  const errorBarData = useMemo(() => buildHistogramErrorBarData(bins, scale), [bins, scale]);
  const relativeErrorData = useMemo(() => buildRelativeErrorStepData(state?.bins), [state?.bins]);
  if (bins.length === 0) return null;
  const xDomain = fitHistogramXDomain(bins);
  const yDomain = buildHistogramYDomain(bins, scale);
  const relativeErrorYDomain = buildRelativeErrorYDomain(relativeErrorData);
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">
            {title}
            {state?.name ? `  (${state.name})` : ""}
          </Typography>
          <FormControl size="small" sx={{ minWidth: 128 }}>
            <Select
              value={scale}
              onChange={(event) => setScale(event.target.value)}
              sx={{
                fontSize: "0.875rem",
                ".MuiSelect-select": { py: 0.75 },
              }}
            >
              <MenuItem value="linear">Linear</MenuItem>
              <MenuItem value="log">Log</MenuItem>
            </Select>
          </FormControl>
        </Box>
        <Box sx={{ width: "100%", display: "grid", gap: 2 }}>
          <Box sx={{ width: "100%", height: 280 }}>
            <ResponsiveContainer width="100%" height="100%">
              <ComposedChart data={stepData} margin={chartMargin}>
                <CartesianGrid stroke={gridColor} vertical={false} />
                <XAxis
                  dataKey="x"
                  type="number"
                  domain={xDomain}
                  tickFormatter={formatAxisValue}
                  tick={axisTickStyle}
                  hide
                />
                <YAxis
                  domain={scale === "log" ? ["auto", "auto"] : yDomain}
                  scale={scale === "log" ? "log" : "linear"}
                  allowDataOverflow={scale === "log"}
                  tickFormatter={formatAxisValue}
                  tick={axisTickStyle}
                  width={72}
                />
                <Tooltip content={<HistogramTooltip />} />
                <Line
                  type="stepAfter"
                  dataKey="y"
                  stroke="#005f73"
                  strokeWidth={1.35}
                  dot={false}
                  isAnimationActive={false}
                />
                <Scatter data={errorBarData} fill="rgba(0,0,0,0)" isAnimationActive={false}>
                  <ErrorBar dataKey="error" width={6} strokeWidth={1.4} stroke="#7c8a96" />
                </Scatter>
              </ComposedChart>
            </ResponsiveContainer>
          </Box>
          <Box>
            <Typography variant="caption" color="text.secondary">
              Relative Error Shape
            </Typography>
            <Box sx={{ width: "100%", height: 168 }}>
              <ResponsiveContainer width="100%" height="100%">
                <ComposedChart data={relativeErrorData} margin={chartMargin}>
                  <CartesianGrid stroke={gridColor} vertical={false} />
                  <XAxis
                    dataKey="x"
                    type="number"
                    domain={xDomain}
                    tickFormatter={formatAxisValue}
                    tick={axisTickStyle}
                  />
                  <YAxis
                    domain={relativeErrorYDomain}
                    tickFormatter={formatAxisValue}
                    tick={axisTickStyle}
                    width={72}
                  />
                  <Tooltip content={<HistogramTooltip />} />
                  <ReferenceLine y={0} stroke="#6b7280" strokeWidth={1} />
                  <Area
                    type="stepAfter"
                    dataKey="positive_relative_error"
                    stroke="#bb3e03"
                    fill="rgba(187, 62, 3, 0.22)"
                    isAnimationActive={false}
                  />
                  <Area
                    type="stepAfter"
                    dataKey="negative_relative_error"
                    stroke="#bb3e03"
                    fill="rgba(187, 62, 3, 0.22)"
                    isAnimationActive={false}
                  />
                </ComposedChart>
              </ResponsiveContainer>
            </Box>
          </Box>
        </Box>
      </CardContent>
    </Card>
  );
};

const TablePanel = ({ title, state }) => {
  const columns = asArray(state?.columns);
  const rows = asArray(state?.rows);
  if (columns.length === 0 || rows.length === 0) return null;
  const payload = state?.payload;
  const selectableRows =
    payload?.histograms && typeof payload.histograms === "object" && !Array.isArray(payload.histograms);
  const bundleJson = selectableRows ? buildHistogramBundleJson(payload) : null;
  const handleDownloadJson = () => {
    const filename = `${state?.panel_id ?? "histogram_bundle"}.json`;
    downloadTextFile(filename, `${JSON.stringify(bundleJson, null, 2)}\n`, "application/json;charset=utf-8");
  };
  const handleDownloadHwU = () => {
    const filename = `${state?.panel_id ?? "histogram_bundle"}.HwU`;
    downloadTextFile(filename, buildHistogramBundleHwU(payload));
  };
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          {selectableRows ? (
            <Stack direction="row" spacing={1} alignItems="center">
              <Button size="small" variant="outlined" onClick={handleDownloadJson}>
                JSON
              </Button>
              <Button size="small" variant="outlined" onClick={handleDownloadHwU}>
                HwU
              </Button>
            </Stack>
          ) : null}
        </Box>
        <TableContainer sx={{ maxHeight: 440, overflowX: "auto" }}>
          <MuiTable size="small" stickyHeader>
            <TableHead>
              <TableRow>
                {columns.map((column) => (
                  <TableCell key={column} sx={{ fontWeight: 600, whiteSpace: "nowrap" }}>
                    {column}
                  </TableCell>
                ))}
              </TableRow>
            </TableHead>
            <TableBody>
              {rows.map((row, rowIndex) => (
                <TableRow
                  key={`row-${rowIndex}`}
                  hover={selectableRows}
                  selected={selectableRows && String(row?.[0] ?? "") === String(state?.selected_value ?? "")}
                  sx={{
                    cursor: selectableRows ? "pointer" : "default",
                  }}
                  onClick={
                    selectableRows && typeof row?.[0] === "string"
                      ? () => state?.onValueChange?.(state?.panel_id, row[0])
                      : undefined
                  }
                >
                  {columns.map((_, columnIndex) => (
                    <TableCell
                      key={`${rowIndex}-${columnIndex}`}
                      sx={{
                        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                        verticalAlign: "top",
                      }}
                    >
                      {renderStructuredValue(row?.[columnIndex])}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </MuiTable>
        </TableContainer>
      </CardContent>
    </Card>
  );
};

const ProgressPanel = ({ title, state }) => {
  const current = Number(state?.current);
  const total = Number(state?.total);
  const progress = Number.isFinite(current) && Number.isFinite(total) && total > 0 ? (current / total) * 100 : 0;
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Typography variant="h5" sx={{ fontFamily: "monospace", mb: 1 }}>
          {Number.isFinite(current) ? current.toLocaleString() : "0"}
          {Number.isFinite(total) ? ` / ${total.toLocaleString()}` : ""}
        </Typography>
        <LinearProgress
          variant={Number.isFinite(total) && total > 0 ? "determinate" : "indeterminate"}
          value={progress}
        />
      </CardContent>
    </Card>
  );
};

const KeyValuePanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 2 }}>
        {title}
      </Typography>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "minmax(0, 1fr)",
            lg: "minmax(0, 1fr) minmax(0, 1fr)",
          },
          gap: 1.5,
        }}
      >
        {asArray(state?.entries).map((entry) => (
          <Box
            key={entry.key}
            sx={{
              display: "grid",
              gridTemplateColumns: "minmax(120px, 0.9fr) minmax(0, 1.1fr)",
              gap: 1,
              py: 0.5,
              borderBottom: "1px solid",
              borderColor: "divider",
            }}
          >
            <Typography variant="body2" color="text.secondary">
              {entry.label}
            </Typography>
            <Typography
              variant="body2"
              sx={{
                fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                wordBreak: "break-word",
                whiteSpace: "pre-wrap",
              }}
            >
              {renderStructuredValue(entry.value)}
            </Typography>
          </Box>
        ))}
      </Box>
    </CardContent>
  </Card>
);

const TextPanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 1 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
        {renderStructuredValue(state?.text)}
      </Typography>
    </CardContent>
  </Card>
);

const TickBreakdownPanel = ({ title, state }) => {
  const totalMs = Number(state?.total_ms);
  const segments = asArray(state?.segments)
    .map((segment) => ({
      ...segment,
      valueMs: Number(segment?.value_ms),
    }))
    .filter((segment) => Number.isFinite(segment.valueMs) && segment.valueMs > 0);
  const normalizedTotal =
    Number.isFinite(totalMs) && totalMs > 0 ? totalMs : segments.reduce((sum, segment) => sum + segment.valueMs, 0);

  if (segments.length === 0 || !Number.isFinite(normalizedTotal) || normalizedTotal <= 0) return null;

  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
            total {formatScientific(normalizedTotal, 4)} ms
          </Typography>
        </Box>
        <Box
          sx={{
            display: "flex",
            width: "100%",
            minHeight: 44,
            borderRadius: 1.5,
            overflow: "hidden",
            border: "1px solid",
            borderColor: "divider",
            backgroundColor: "rgba(15,23,42,0.04)",
          }}
        >
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            const showInlineLabel = percent >= 10;
            return (
              <Box
                key={segment.key}
                title={`${segment.label}: ${formatScientific(segment.valueMs, 4)} ms (${formatScientific(percent, 3)}%)`}
                sx={{
                  width: `${Math.max(percent, 1.5)}%`,
                  minWidth: 0,
                  px: showInlineLabel ? 1 : 0,
                  py: 0.75,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: showInlineLabel ? "space-between" : "center",
                  gap: 1,
                  color: "#fff",
                  backgroundColor: segment.color || "#0f766e",
                }}
              >
                {showInlineLabel ? (
                  <>
                    <Typography variant="caption" sx={{ fontWeight: 600, color: "inherit", lineHeight: 1.15 }}>
                      {segment.label}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "inherit", opacity: 0.95, lineHeight: 1.15 }}>
                      {formatScientific(segment.valueMs, 3)} ms
                    </Typography>
                  </>
                ) : null}
              </Box>
            );
          })}
        </Box>
        <Box
          sx={{
            mt: 1.25,
            display: "grid",
            gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
            gap: 0.75,
          }}
        >
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            return (
              <Box key={`${segment.key}-legend`} sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Box
                  sx={{
                    width: 10,
                    height: 10,
                    borderRadius: 0.5,
                    backgroundColor: segment.color || "#0f766e",
                    flexShrink: 0,
                  }}
                />
                <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0 }}>
                  {segment.label}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{ ml: "auto", fontFamily: "monospace", color: "text.secondary", whiteSpace: "nowrap" }}
                >
                  {formatScientific(segment.valueMs, 4)} ms ({formatScientific(percent, 3)}%)
                </Typography>
              </Box>
            );
          })}
        </Box>
      </CardContent>
    </Card>
  );
};

const hsvToRgb = (h, s, v) => {
  const c = v * s;
  const hh = (((h % 360) + 360) % 360) / 60;
  const x = c * (1 - Math.abs((hh % 2) - 1));
  let rgb = [0, 0, 0];
  if (hh < 1) rgb = [c, x, 0];
  else if (hh < 2) rgb = [x, c, 0];
  else if (hh < 3) rgb = [0, c, x];
  else if (hh < 4) rgb = [0, x, c];
  else if (hh < 5) rgb = [x, 0, c];
  else rgb = [c, 0, x];
  const m = v - c;
  return rgb.map((value) => Math.round((value + m) * 255));
};

const scalarImageLegendGradient = "linear-gradient(to top, rgb(0,0,255), rgb(128,200,128), rgb(255,0,0))";

const ComplexImageColorbar = ({ colorMode, normalizationMode, complexMaxMagnitude, scalarMin, scalarMax }) => {
  const isHueIntensity = colorMode === "complex_hue_intensity";

  return (
    <Box
      sx={{
        width: 80,
        minWidth: 80,
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        gap: 1,
      }}
    >
      <Typography variant="body2" color="text.secondary">
        {isHueIntensity ? "Magnitude" : "Value"}
      </Typography>
      <Box
        sx={{
          height: 240,
          borderRadius: 1,
          border: "1px solid",
          borderColor: "divider",
          background: isHueIntensity
            ? "linear-gradient(to top, rgb(0,0,0), rgb(255,255,255))"
            : scalarImageLegendGradient,
        }}
      />
      <Typography variant="caption" sx={{ fontFamily: "monospace" }}>
        {formatScientific(isHueIntensity ? complexMaxMagnitude : scalarMax, 4)}
      </Typography>
      {isHueIntensity ? null : normalizationMode === "symmetric" ? (
        <Typography variant="caption" color="text.secondary" sx={{ fontFamily: "monospace" }}>
          0
        </Typography>
      ) : null}
      <Typography variant="caption" color="text.secondary" sx={{ fontFamily: "monospace" }}>
        {formatScientific(isHueIntensity ? 0 : scalarMin, 4)}
      </Typography>
    </Box>
  );
};

const SelectPanel = ({ title, descriptor, value, onValueChange }) => {
  const options = asArray(descriptor?.state?.options);
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <FormControl fullWidth size="small">
          <Select value={value ?? ""} onChange={(event) => onValueChange?.(descriptor.panel_id, event.target.value)}>
            {options.map((option) => (
              <MenuItem key={String(option.value)} value={option.value}>
                {option.label}
              </MenuItem>
            ))}
          </Select>
        </FormControl>
      </CardContent>
    </Card>
  );
};

const Image2dPanel = ({ title, state }) => {
  const canvasRef = useRef(null);
  const width = Number(state?.width) || 0;
  const height = Number(state?.height) || 0;
  const values = useMemo(() => asArray(state?.values), [state?.values]);
  const imagValues = useMemo(() => {
    const next = asArray(state?.imag_values);
    return next.length > 0 ? next : null;
  }, [state?.imag_values]);
  const invalidIndices = useMemo(() => new Set(asArray(state?.invalid_indices)), [state?.invalid_indices]);
  const colorMode = state?.color_mode || "scalar_heatmap";
  const normalizationMode = state?.normalization_mode || "min_max";
  const xRange = useMemo(() => asArray(state?.x_range), [state?.x_range]);
  const yRange = useMemo(() => asArray(state?.y_range), [state?.y_range]);
  const [hover, setHover] = useState(null);

  const useScalarHeatmap = !imagValues && colorMode === "scalar_heatmap";
  const { zmin: scalarMin, zmax: scalarMax } = useMemo(
    () => buildScalarHeatmapScale(values, normalizationMode),
    [normalizationMode, values],
  );
  const complexMagnitudes = useMemo(
    () => (imagValues ? values.map((re, index) => Math.hypot(re, imagValues[index] || 0)) : []),
    [imagValues, values],
  );
  const complexMaxMagnitude = useMemo(() => Math.max(...complexMagnitudes, 1e-12), [complexMagnitudes]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || useScalarHeatmap || width <= 0 || height <= 0 || values.length === 0) return;
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const image = ctx.createImageData(width, height);

    if (imagValues && colorMode === "complex_hue_intensity") {
      for (let index = 0; index < values.length; index += 1) {
        const offset = index * 4;
        if (invalidIndices.has(index)) {
          image.data[offset] = invalidImageColor[0];
          image.data[offset + 1] = invalidImageColor[1];
          image.data[offset + 2] = invalidImageColor[2];
          image.data[offset + 3] = 255;
          continue;
        }
        const re = values[index];
        const im = imagValues[index] || 0;
        if (!Number.isFinite(re) || !Number.isFinite(im)) {
          image.data[offset + 3] = 0;
          continue;
        }
        const phase = (Math.atan2(im, re) / Math.PI) * 180 + 180;
        const magnitude = Math.hypot(re, im) / complexMaxMagnitude;
        const [r, g, b] = hsvToRgb(phase, 1, Math.min(1, Math.sqrt(magnitude)));
        image.data[offset] = r;
        image.data[offset + 1] = g;
        image.data[offset + 2] = b;
        image.data[offset + 3] = 255;
      }
    } else {
      const finite = values.filter((value) => Number.isFinite(value));
      const min =
        normalizationMode === "symmetric"
          ? -(finite.length > 0 ? Math.max(...finite.map((value) => Math.abs(value))) : 1)
          : finite.length > 0
            ? Math.min(...finite)
            : 0;
      const max =
        normalizationMode === "symmetric"
          ? finite.length > 0
            ? Math.max(...finite.map((value) => Math.abs(value)))
            : 1
          : finite.length > 0
            ? Math.max(...finite)
            : 1;
      const span = max - min || 1;
      for (let index = 0; index < values.length; index += 1) {
        const offset = index * 4;
        if (invalidIndices.has(index)) {
          image.data[offset] = invalidImageColor[0];
          image.data[offset + 1] = invalidImageColor[1];
          image.data[offset + 2] = invalidImageColor[2];
          image.data[offset + 3] = 255;
          continue;
        }
        const value = values[index];
        if (!Number.isFinite(value)) {
          image.data[offset + 3] = 0;
          continue;
        }
        const t = Number.isFinite(value) ? (value - min) / span : 0;
        const r = Math.round(255 * t);
        const g = Math.round(200 * (1 - Math.abs(t - 0.5) * 2));
        const b = Math.round(255 * (1 - t));
        image.data[offset] = r;
        image.data[offset + 1] = g;
        image.data[offset + 2] = b;
        image.data[offset + 3] = 255;
      }
    }

    ctx.putImageData(image, 0, 0);
  }, [
    colorMode,
    complexMaxMagnitude,
    height,
    imagValues,
    invalidIndices,
    normalizationMode,
    useScalarHeatmap,
    values,
    width,
  ]);

  if (width <= 0 || height <= 0 || values.length === 0) return null;

  if (useScalarHeatmap) {
    return (
      <ScalarImageHeatmapPanel
        title={title}
        width={width}
        height={height}
        values={values}
        invalidIndices={invalidIndices}
        normalizationMode={normalizationMode}
        xRange={xRange}
        yRange={yRange}
      />
    );
  }

  const [xMin, xMax] = xRange;
  const [yMin, yMax] = yRange;
  const xStep = width > 0 && Number.isFinite(xMin) && Number.isFinite(xMax) ? (xMax - xMin) / width : 1;
  const yStep = height > 0 && Number.isFinite(yMin) && Number.isFinite(yMax) ? (yMax - yMin) / height : 1;

  const handleCanvasHover = (event) => {
    const canvas = canvasRef.current;
    if (!canvas || !imagValues) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const col = Math.min(width - 1, Math.max(0, Math.floor(((event.clientX - rect.left) / rect.width) * width)));
    const row = Math.min(height - 1, Math.max(0, Math.floor(((event.clientY - rect.top) / rect.height) * height)));
    const index = row * width + col;
    if (invalidIndices.has(index)) {
      setHover(null);
      return;
    }
    const re = values[index];
    const im = imagValues[index] || 0;
    if (!Number.isFinite(re) || !Number.isFinite(im)) {
      setHover(null);
      return;
    }
    setHover({
      left: event.clientX - rect.left,
      top: event.clientY - rect.top,
      x: Number.isFinite(xMin) && Number.isFinite(xMax) ? xMin + xStep * (col + 0.5) : col,
      y: Number.isFinite(yMin) && Number.isFinite(yMax) ? yMin + yStep * (row + 0.5) : row,
      re,
      im,
      magnitude: Math.hypot(re, im),
      phase: Math.atan2(im, re),
    });
  };

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Box
          sx={{
            width: "100%",
            display: "flex",
            justifyContent: "center",
            alignItems: "stretch",
            gap: 2,
            overflow: "auto",
          }}
        >
          <Box
            sx={{
              position: "relative",
              width: "100%",
              maxWidth: 640,
            }}
            onMouseMove={handleCanvasHover}
            onMouseLeave={() => setHover(null)}
          >
            <Box
              component="canvas"
              ref={canvasRef}
              sx={{
                width: "100%",
                display: "block",
                imageRendering: "pixelated",
                border: "1px solid",
                borderColor: "divider",
              }}
            />
            <ComplexImageTooltip hover={hover} />
          </Box>
          {imagValues ? (
            <ComplexImageColorbar
              colorMode={colorMode}
              normalizationMode={normalizationMode}
              complexMaxMagnitude={complexMaxMagnitude}
              scalarMin={scalarMin}
              scalarMax={scalarMax}
            />
          ) : null}
        </Box>
      </CardContent>
    </Card>
  );
};

const PanelRenderer = ({ descriptor, state, value, onValueChange }) => {
  if (!descriptor) return null;
  switch (descriptor.kind) {
    case "select":
      return (
        <SelectPanel title={descriptor.label} descriptor={descriptor} value={value} onValueChange={onValueChange} />
      );
    case "scalar_timeseries":
      if (!state) return null;
      return <ScalarTimeseriesPanel title={descriptor.label} state={state} />;
    case "multi_timeseries":
      if (!state) return null;
      return <MultiTimeseriesPanel title={descriptor.label} state={state} />;
    case "tick_breakdown":
      if (!state) return null;
      return <TickBreakdownPanel title={descriptor.label} state={state} />;
    case "progress":
      if (!state) return null;
      return <ProgressPanel title={descriptor.label} state={state} />;
    case "key_value":
      if (!state) return null;
      return <KeyValuePanel title={descriptor.label} state={state} />;
    case "image2d":
      if (!state) return null;
      return <Image2dPanel title={descriptor.label} state={state} />;
    case "table":
      if (!state) return null;
      return (
        <TablePanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id, selected_value: value, onValueChange }}
        />
      );
    case "histogram":
      if (!state) return null;
      return <HistogramPanel title={descriptor.label} state={state} />;
    case "text":
      if (!state) return null;
      return <TextPanel title={descriptor.label} state={state} />;
    default:
      return null;
  }
};

const PanelCollection = ({ title = null, panelSpecs, panelStates, panelValues = {}, onPanelValueChange = null }) => {
  const renderablePanels = useMemo(
    () => buildRenderablePanels(panelSpecs, panelStates, panelValues),
    [panelSpecs, panelStates, panelValues],
  );

  return (
    <Box sx={{ mb: 3 }}>
      {title ? (
        <Typography variant="h6" sx={{ mb: 2 }}>
          {title}
        </Typography>
      ) : null}
      {renderablePanels.length === 0 ? <Alert severity="info">No panel data available yet.</Alert> : null}
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "minmax(0, 1fr)",
            md: "repeat(12, minmax(0, 1fr))",
          },
          gap: 2,
          alignItems: "start",
        }}
      >
        {renderablePanels.map(({ descriptor, state, value }) => (
          <Box
            key={descriptor.panel_id}
            sx={{
              minWidth: 0,
              gridColumn: panelColumnSpan(descriptor),
            }}
          >
            <PanelRenderer descriptor={descriptor} state={state} value={value} onValueChange={onPanelValueChange} />
          </Box>
        ))}
      </Box>
    </Box>
  );
};

export default PanelCollection;
