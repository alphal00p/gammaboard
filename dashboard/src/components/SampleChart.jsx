import { Box, Card, CardContent, Typography, Paper } from "@mui/material";
import {
  LineChart,
  Area,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  CartesianGrid,
  ResponsiveContainer,
  Legend,
  ReferenceLine,
} from "recharts";
import { TrendingUp as TrendingUpIcon } from "@mui/icons-material";
import { formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";

const SampleChart = ({ samples, isConnected, hasRun, mode = "scalar", target = null }) => {
  const isComplex = mode === "complex";
  const meanLabel = isComplex ? "Value" : "Mean";
  const chartTitle = isComplex ? "Complex Observable Convergence" : "Integration Mean Convergence";

  const formatYAxisTick = (value) => {
    return formatScientific(value, 2, "");
  };

  const percentile = (sortedValues, p) => {
    if (!sortedValues.length) return 0;
    if (sortedValues.length === 1) return sortedValues[0];
    const clamped = Math.max(0, Math.min(1, p));
    const idx = clamped * (sortedValues.length - 1);
    const lo = Math.floor(idx);
    const hi = Math.ceil(idx);
    if (lo === hi) return sortedValues[lo];
    const t = idx - lo;
    return sortedValues[lo] * (1 - t) + sortedValues[hi] * t;
  };

  const buildChartData = (samples, chartMode) => {
    if (!samples || samples.length === 0) return [];

    const isComplexData = chartMode === "complex";
    return samples
      .filter((sample) => Number.isFinite(sample.sampleCount))
      .map((sample) => {
        if (isComplexData) {
          const real = Number(sample.real);
          const imag = Number(sample.imag);
          return {
            sampleCount: sample.sampleCount,
            real: Number.isFinite(real) ? real : 0,
            imag: Number.isFinite(imag) ? imag : 0,
          };
        }

        return {
          sampleCount: sample.sampleCount,
          mean: sample.mean ?? sample.value ?? 0,
          stderr: Number.isFinite(sample.stderr) ? sample.stderr : 0,
          lower: Number.isFinite(sample.lower) ? sample.lower : (sample.mean ?? sample.value ?? 0),
          upper: Number.isFinite(sample.upper) ? sample.upper : (sample.mean ?? sample.value ?? 0),
          spread: Number.isFinite(sample.spread) ? sample.spread : 0,
        };
      })
      .sort((a, b) => a.sampleCount - b.sampleCount);
  };

  const CustomTooltip = ({ active, payload }) => {
    if (!active || !payload || payload.length === 0) return null;

    const data = payload[0].payload;

    return (
      <Paper elevation={3} sx={{ p: 1.5, border: 1, borderColor: "divider" }}>
        <Box sx={{ display: "flex", flexDirection: "column", gap: 0.5 }}>
          <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
            <Typography variant="caption" color="text.secondary">
              Samples:
            </Typography>
            <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
              {data.sampleCount}
            </Typography>
          </Box>

          <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
            <Typography variant="caption" color="text.secondary">
              {isComplex ? "Real Mean:" : `${meanLabel}:`}
            </Typography>
            <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
              {formatScientific(isComplex ? data.real : data.mean, 6)}
            </Typography>
          </Box>

          {!isComplex && Number.isFinite(data.stderr) && (
            <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
              <Typography variant="caption" color="text.secondary">
                Std. Error:
              </Typography>
              <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
                {formatScientific(data.stderr, 6)}
              </Typography>
            </Box>
          )}

          {isComplex && (
            <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
              <Typography variant="caption" color="text.secondary">
                Imag Mean:
              </Typography>
              <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
                {formatScientific(data.imag, 6)}
              </Typography>
            </Box>
          )}
        </Box>
      </Paper>
    );
  };

  const EmptyState = () => {
    const message =
      isConnected && hasRun ? "No aggregated results yet. Waiting for updates..." : "Select a run to view data";

    return (
      <Box
        sx={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          minHeight: 400,
          textAlign: "center",
          p: 4,
          border: 2,
          borderStyle: "dashed",
          borderColor: "divider",
          borderRadius: 2,
          backgroundColor: "background.default",
        }}
      >
        <TrendingUpIcon sx={{ fontSize: 60, color: "action.disabled", mb: 2 }} />
        <Typography variant="body1" color="text.secondary">
          {message}
        </Typography>
      </Box>
    );
  };

  if (!samples || samples.length === 0) {
    return (
      <Box sx={{ mb: 3 }}>
        <Typography variant="h6" gutterBottom>
          {isComplex ? "Complex Observable (Aggregated)" : "Integration Mean (Aggregated)"}
        </Typography>
        <EmptyState />
      </Box>
    );
  }

  const chartData = buildChartData(samples, mode);
  const lastPoint = chartData.length > 0 ? chartData[chartData.length - 1] : null;
  const currentMean = lastPoint ? lastPoint.mean : 0;
  const currentReal = lastPoint ? lastPoint.real : 0;
  const currentImag = lastPoint ? lastPoint.imag : 0;
  const scalarTarget = !isComplex ? parseScalarTarget(target) : null;
  const targetValue = scalarTarget?.value ?? null;
  const currentDeltaToTarget = targetValue != null ? currentMean - targetValue : null;
  const xMin = chartData[0]?.sampleCount ?? 0;
  const xMax = chartData[chartData.length - 1]?.sampleCount ?? 0;
  const xDomain = xMin === xMax ? [xMin - 1, xMax + 1] : [xMin, xMax];

  const domainWindowSize = 120;
  const domainData = chartData.slice(-domainWindowSize);
  const yValues = isComplex
    ? domainData.flatMap((d) => [d.real, d.imag])
    : domainData.flatMap((d) => [d.mean, d.lower, d.upper]);
  if (targetValue != null) {
    yValues.push(targetValue);
  }
  const sortedY = [...yValues].sort((a, b) => a - b);
  const qLow = percentile(sortedY, 0.05);
  const qHigh = percentile(sortedY, 0.95);
  const latestValues = isComplex
    ? [currentReal, currentImag]
    : [currentMean, lastPoint?.lower ?? currentMean, lastPoint?.upper ?? currentMean];
  const hardMin = Math.min(...latestValues, ...(targetValue != null ? [targetValue] : []));
  const hardMax = Math.max(...latestValues, ...(targetValue != null ? [targetValue] : []));
  const yMin = Math.min(qLow, hardMin);
  const yMax = Math.max(qHigh, hardMax);
  const yAbsMax = Math.max(Math.abs(yMin), Math.abs(yMax));
  const yPadding =
    yMin === yMax ? Math.max(Math.abs(yMin) * 0.005, 1e-14) : Math.max((yMax - yMin) * 0.003, yAbsMax * 5e-8, 1e-14);
  const yDomain = [yMin - yPadding, yMax + yPadding];

  return (
    <Box sx={{ mb: 3 }}>
      <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 1 }}>
        <Typography variant="h6" sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <TrendingUpIcon />
          {chartTitle}
        </Typography>
        <Typography variant="h5" sx={{ fontWeight: 700, fontFamily: "monospace", color: "primary.main" }}>
          {isComplex
            ? `Re ${formatScientific(currentReal, 6)} | Im ${formatScientific(currentImag, 6)}`
            : formatScientific(currentMean, 6)}
        </Typography>
      </Box>
      {!isComplex && targetValue != null && (
        <Typography variant="body2" sx={{ mb: 1, fontFamily: "monospace" }} color="text.secondary">
          target {formatScientific(targetValue, 6)} | delta {formatScientific(currentDeltaToTarget, 6)}
        </Typography>
      )}

      <Card>
        <CardContent>
          <ResponsiveContainer width="100%" height={500}>
            <LineChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 25 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#e0e0e0" />
              <XAxis
                dataKey="sampleCount"
                type="number"
                domain={xDomain}
                label={{ value: "Sample Count", position: "insideBottom", offset: -10 }}
                stroke="#666"
              />
              <YAxis
                domain={yDomain}
                tickFormatter={formatYAxisTick}
                tickCount={6}
                label={{ value: meanLabel, angle: -90, position: "insideLeft" }}
                stroke="#666"
              />
              <Tooltip content={<CustomTooltip />} />
              {isComplex && <Legend verticalAlign="top" height={28} />}
              {isComplex ? (
                <>
                  <Line
                    type="monotone"
                    dataKey="real"
                    name="Real"
                    stroke="#1976d2"
                    strokeWidth={2}
                    dot={false}
                    isAnimationActive={false}
                  />
                  <Line
                    type="monotone"
                    dataKey="imag"
                    name="Imaginary"
                    stroke="#ef6c00"
                    strokeWidth={2}
                    dot={false}
                    isAnimationActive={false}
                  />
                </>
              ) : (
                <>
                  {targetValue != null && (
                    <ReferenceLine
                      y={targetValue}
                      stroke="#2e7d32"
                      strokeWidth={1.5}
                      strokeDasharray="6 4"
                      ifOverflow="extendDomain"
                      label={{ value: "target", position: "insideTopRight", fill: "#2e7d32" }}
                    />
                  )}
                  <Area
                    type="monotone"
                    dataKey="lower"
                    stackId="error-band"
                    stroke="none"
                    fill="transparent"
                    isAnimationActive={false}
                  />
                  <Area
                    type="monotone"
                    dataKey="spread"
                    stackId="error-band"
                    stroke="none"
                    fill="#1976d2"
                    fillOpacity={0.18}
                    isAnimationActive={false}
                  />
                  <Line
                    type="monotone"
                    dataKey="mean"
                    stroke="#1976d2"
                    strokeWidth={2}
                    dot={false}
                    isAnimationActive={false}
                  />
                </>
              )}
            </LineChart>
          </ResponsiveContainer>

          <Box sx={{ mt: 2, pt: 2, borderTop: 1, borderColor: "divider" }}>
            <Typography variant="caption" color="text.secondary">
              Showing convergence over {samples.length.toLocaleString()} snapshots
            </Typography>
          </Box>
        </CardContent>
      </Card>
    </Box>
  );
};

export default SampleChart;
