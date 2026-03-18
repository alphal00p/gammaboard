import { useEffect, useMemo, useRef } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  LinearProgress,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableRow,
  Typography,
} from "@mui/material";
import { Area, CartesianGrid, ComposedChart, Line, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { formatScientific } from "../../utils/formatters";
import { asArray } from "../../utils/collections";

const aggregatePanelHistory = (items) => {
  const panels = new Map();
  for (const item of asArray(items)) {
    for (const panel of asArray(item?.panels)) {
      if (!panel?.panel_id || !panel?.kind) continue;
      if (panel.kind === "scalar_timeseries") {
        const existing = panels.get(panel.panel_id) || { ...panel, points: [] };
        existing.points = [...existing.points, ...asArray(panel.points)];
        panels.set(panel.panel_id, existing);
      } else if (panel.kind === "multi_timeseries") {
        const existing = panels.get(panel.panel_id) || { ...panel, series: [] };
        const seriesMap = new Map((existing.series || []).map((series) => [series.id, { ...series }]));
        for (const series of asArray(panel.series)) {
          const previous = seriesMap.get(series.id) || { ...series, points: [] };
          previous.points = [...previous.points, ...asArray(series.points)];
          seriesMap.set(series.id, previous);
        }
        existing.series = Array.from(seriesMap.values());
        panels.set(panel.panel_id, existing);
      } else {
        panels.set(panel.panel_id, panel);
      }
    }
  }
  return panels;
};

const buildRenderablePanels = (descriptors, currentPanels, historyItems) => {
  const currentMap = new Map(asArray(currentPanels).map((panel) => [panel.panel_id, panel]));
  const historyMap = aggregatePanelHistory(historyItems);
  return asArray(descriptors).map((descriptor) => ({
    descriptor,
    state:
      (descriptor.supports_history &&
        (descriptor.kind === "scalar_timeseries" || descriptor.kind === "multi_timeseries"
          ? historyMap.get(descriptor.panel_id)
          : null)) ||
      currentMap.get(descriptor.panel_id) ||
      historyMap.get(descriptor.panel_id) ||
      null,
  }));
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
    return [min, max];
  }
  return [min, max];
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
      <Table size="small">
        <TableBody>
          {asArray(state?.entries).map((entry) => (
            <TableRow key={entry.key}>
              <TableCell>{entry.label}</TableCell>
              <TableCell sx={{ fontFamily: "monospace" }}>
                {typeof entry.value === "number" ? formatScientific(entry.value, 6) : JSON.stringify(entry.value)}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </CardContent>
  </Card>
);

const TextPanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 1 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary">
        {state?.text || "No data"}
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

const Image2dPanel = ({ title, state }) => {
  const canvasRef = useRef(null);
  const width = Number(state?.width) || 0;
  const height = Number(state?.height) || 0;
  const values = useMemo(() => asArray(state?.values), [state?.values]);
  const imagValues = useMemo(() => {
    const next = asArray(state?.imag_values);
    return next.length > 0 ? next : null;
  }, [state?.imag_values]);
  const colorMode = state?.color_mode || "scalar_heatmap";

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
        const re = values[index];
        const im = imagValues[index] || 0;
        const phase = (Math.atan2(im, re) / Math.PI) * 180 + 180;
        const magnitude = Math.hypot(re, im) / maxMagnitude;
        const [r, g, b] = hsvToRgb(phase, 1, Math.min(1, Math.sqrt(magnitude)));
        const offset = index * 4;
        image.data[offset] = r;
        image.data[offset + 1] = g;
        image.data[offset + 2] = b;
        image.data[offset + 3] = 255;
      }
    } else {
      const finite = values.filter((value) => Number.isFinite(value));
      const min = finite.length > 0 ? Math.min(...finite) : 0;
      const max = finite.length > 0 ? Math.max(...finite) : 1;
      const span = max - min || 1;
      for (let index = 0; index < values.length; index += 1) {
        const value = values[index];
        const t = Number.isFinite(value) ? (value - min) / span : 0;
        const r = Math.round(255 * t);
        const g = Math.round(200 * (1 - Math.abs(t - 0.5) * 2));
        const b = Math.round(255 * (1 - t));
        const offset = index * 4;
        image.data[offset] = r;
        image.data[offset + 1] = g;
        image.data[offset + 2] = b;
        image.data[offset + 3] = 255;
      }
    }

    ctx.putImageData(image, 0, 0);
  }, [colorMode, height, imagValues, values, width]);

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

const PanelRenderer = ({ descriptor, state }) => {
  if (!descriptor || !state) return null;
  switch (descriptor.kind) {
    case "scalar_timeseries":
      return <ScalarTimeseriesPanel title={descriptor.label} state={state} />;
    case "multi_timeseries":
      return <MultiTimeseriesPanel title={descriptor.label} state={state} />;
    case "progress":
      return <ProgressPanel title={descriptor.label} state={state} />;
    case "key_value":
      return <KeyValuePanel title={descriptor.label} state={state} />;
    case "image2d":
      return <Image2dPanel title={descriptor.label} state={state} />;
    case "text":
      return <TextPanel title={descriptor.label} state={state} />;
    default:
      return null;
  }
};

const PanelCollection = ({ title = null, descriptors, currentPanels, historyItems }) => {
  const renderablePanels = useMemo(
    () => buildRenderablePanels(descriptors, currentPanels, historyItems),
    [currentPanels, descriptors, historyItems],
  );

  return (
    <Box sx={{ mb: 3 }}>
      {title ? (
        <Typography variant="h6" sx={{ mb: 2 }}>
          {title}
        </Typography>
      ) : null}
      {renderablePanels.length === 0 ? <Alert severity="info">No panel data available yet.</Alert> : null}
      <Stack spacing={2}>
        {renderablePanels.map(({ descriptor, state }) => (
          <Box key={descriptor.panel_id}>
            <PanelRenderer descriptor={descriptor} state={state} />
          </Box>
        ))}
      </Stack>
    </Box>
  );
};

export default PanelCollection;
