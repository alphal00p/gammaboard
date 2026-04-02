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
import {
  Area,
  CartesianGrid,
  ComposedChart,
  ErrorBar,
  Line,
  ResponsiveContainer,
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

const formatAxisNumber = (value) => formatScientific(value, 2, "");

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

const buildScalarBandData = (points) =>
  points.map((point) => ({
    ...point,
    band_lower: Number.isFinite(point?.y_min) ? point.y_min : null,
    band_upper_delta: Number.isFinite(point?.y_min) && Number.isFinite(point?.y_max) ? point.y_max - point.y_min : null,
  }));

const ScalarTimeseriesPanel = ({ title, state }) => {
  const points = asArray(state?.points)
    .slice()
    .sort((a, b) => a.x - b.x);
  if (points.length === 0) return null;
  const data = buildScalarBandData(points);
  const domain = fitDomain(data.flatMap((point) => [point.y, point.y_min, point.y_max]));
  const xDomain = fitXDomain(data.map((point) => point.x));
  const hasBand = data.some((point) => Number.isFinite(point.y_min) && Number.isFinite(point.y_max));
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <ResponsiveContainer width="100%" height={280}>
          <ComposedChart data={data}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="x" type="number" domain={xDomain} allowDataOverflow />
            <YAxis tickFormatter={formatAxisNumber} domain={domain} allowDataOverflow />
            <Tooltip formatter={(value) => formatScientific(value, 6)} />
            {hasBand ? (
              <>
                <Area
                  type="monotone"
                  dataKey="band_lower"
                  stackId="band"
                  stroke="none"
                  fill="transparent"
                  isAnimationActive={false}
                  activeDot={false}
                />
                <Area
                  type="monotone"
                  dataKey="band_upper_delta"
                  stackId="band"
                  stroke="none"
                  fill={bandColor}
                  isAnimationActive={false}
                  activeDot={false}
                />
              </>
            ) : null}
            <Line type="monotone" dataKey="y" stroke="#005f73" dot={false} isAnimationActive={false} />
          </ComposedChart>
        </ResponsiveContainer>
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
        <ResponsiveContainer width="100%" height={280}>
          <ComposedChart data={data}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="x" type="number" domain={xDomain} allowDataOverflow />
            <YAxis tickFormatter={formatAxisNumber} domain={domain} allowDataOverflow />
            <Tooltip formatter={(value) => formatScientific(value, 6)} />
            {series.map((item, index) => (
              <Line
                key={item.id}
                type="monotone"
                dataKey={item.id}
                name={item.label}
                stroke={lineColors[index % lineColors.length]}
                dot={false}
                isAnimationActive={false}
              />
            ))}
          </ComposedChart>
        </ResponsiveContainer>
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

const HistogramPanel = ({ title, state }) => {
  const [scale, setScale] = useState("linear");
  const bins = useMemo(() => buildHistogramData(state?.bins), [state?.bins]);
  const stepData = useMemo(() => buildHistogramRenderData(state?.bins, scale), [scale, state?.bins]);
  if (bins.length === 0) return null;
  const xDomain = fitHistogramXDomain(bins);
  const yDomain = buildHistogramYDomain(bins, scale);
  const yScale = scale === "log" && yDomain[0] !== "auto" ? "log" : "auto";
  const yTickFormatter = scale === "log" ? (value) => formatScientific(value, 2, "") : formatAxisNumber;
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
        <ResponsiveContainer width="100%" height={280}>
          <ComposedChart data={stepData}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis dataKey="x" type="number" domain={xDomain} allowDataOverflow tickFormatter={formatAxisNumber} />
            <YAxis tickFormatter={yTickFormatter} domain={yDomain} allowDataOverflow scale={yScale} width={72} />
            <Tooltip
              formatter={(value, name, props) => {
                if (name === "y") {
                  const error = Number(props?.payload?.error) || 0;
                  return [`${formatScientific(value, 6)} ± ${formatScientific(error, 6)}`, "bin average"];
                }
                return [formatScientific(value, 6), name];
              }}
              labelFormatter={(_, payload) => payload?.[0]?.payload?.rangeLabel || ""}
            />
            <Line
              type="stepAfter"
              dataKey="y"
              stroke="#005f73"
              strokeWidth={1.35}
              dot={false}
              activeDot={{ r: 2.5, fill: "#005f73", stroke: "#f8fafc", strokeWidth: 1 }}
              isAnimationActive={false}
            >
              <ErrorBar
                dataKey="error"
                direction="y"
                stroke="#7c8a96"
                strokeWidth={1.4}
                width={6}
                isAnimationActive={false}
                animationDuration={0}
              />
            </Line>
          </ComposedChart>
        </ResponsiveContainer>
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
  const invalidColor = [255, 0, 255];

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || width <= 0 || height <= 0 || values.length === 0) return;
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const image = ctx.createImageData(width, height);

    if (imagValues && colorMode === "complex_hue_intensity") {
      const magnitudes = values.map((re, index) => Math.hypot(re, imagValues[index] || 0));
      const maxMagnitude = Math.max(...magnitudes, 1e-12);
      for (let index = 0; index < values.length; index += 1) {
        const offset = index * 4;
        if (invalidIndices.has(index)) {
          image.data[offset] = invalidColor[0];
          image.data[offset + 1] = invalidColor[1];
          image.data[offset + 2] = invalidColor[2];
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
        const magnitude = Math.hypot(re, im) / maxMagnitude;
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
          image.data[offset] = invalidColor[0];
          image.data[offset + 1] = invalidColor[1];
          image.data[offset + 2] = invalidColor[2];
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
  }, [colorMode, height, imagValues, invalidIndices, normalizationMode, values, width]);

  if (width <= 0 || height <= 0 || values.length === 0) return null;

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
            overflow: "auto",
          }}
        >
          <Box
            component="canvas"
            ref={canvasRef}
            sx={{
              width: "100%",
              maxWidth: 640,
              imageRendering: "pixelated",
              border: "1px solid",
              borderColor: "divider",
            }}
          />
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
