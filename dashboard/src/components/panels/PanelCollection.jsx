import { useEffect, useMemo, useRef } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  FormControl,
  LinearProgress,
  MenuItem,
  Select,
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

const buildRenderablePanels = (panelSpecs, panelStates, panelValues) => {
  const stateMap = new Map(asArray(panelStates).map((panel) => [panel.panel_id, panel]));
  return asArray(panelSpecs).map((spec) => ({
    descriptor: spec,
    state: stateMap.get(spec.panel_id) || null,
    value: panelValues?.[spec.panel_id],
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
  const colorMode = state?.color_mode || "scalar_heatmap";
  const normalizationMode = state?.normalization_mode || "min_max";

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
  }, [colorMode, height, imagValues, normalizationMode, values, width]);

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
      <Stack spacing={2}>
        {renderablePanels.map(({ descriptor, state, value }) => (
          <Box key={descriptor.panel_id}>
            <PanelRenderer descriptor={descriptor} state={state} value={value} onValueChange={onPanelValueChange} />
          </Box>
        ))}
      </Stack>
    </Box>
  );
};

export default PanelCollection;
