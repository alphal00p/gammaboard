import { memo, useMemo } from "react";
import { Box, Card, CardContent, Typography, Paper } from "@mui/material";
import {
  ComposedChart,
  Area,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  CartesianGrid,
  ResponsiveContainer,
  ReferenceLine,
} from "recharts";
import { TrendingUp as TrendingUpIcon } from "@mui/icons-material";
import { formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";

const buildChartData = (samples) => {
  if (!samples || samples.length === 0) return [];
  return samples
    .filter((sample) => Number.isFinite(sample.sampleCount))
    .map((sample) => ({
      sampleCount: sample.sampleCount,
      mean: sample.mean ?? sample.value ?? 0,
      stderr: Number.isFinite(sample.stderr) ? sample.stderr : 0,
      lower: Number.isFinite(sample.lower) ? sample.lower : (sample.mean ?? sample.value ?? 0),
      upper: Number.isFinite(sample.upper) ? sample.upper : (sample.mean ?? sample.value ?? 0),
      spread: Number.isFinite(sample.spread) ? sample.spread : 0,
    }))
    .sort((a, b) => a.sampleCount - b.sampleCount);
};

const ScalarTooltip = ({ active, payload, sampleLabel, valueLabel, showStdErr }) => {
  if (!active || !payload || payload.length === 0) return null;
  const data = payload[0].payload;
  return (
    <Paper elevation={3} sx={{ p: 1.5, border: 1, borderColor: "divider" }}>
      <Box sx={{ display: "flex", flexDirection: "column", gap: 0.5 }}>
        <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
          <Typography variant="caption" color="text.secondary">
            {sampleLabel}:
          </Typography>
          <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
            {data.sampleCount}
          </Typography>
        </Box>
        <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
          <Typography variant="caption" color="text.secondary">
            {valueLabel}:
          </Typography>
          <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
            {formatScientific(data.mean, 6)}
          </Typography>
        </Box>
        {showStdErr && Number.isFinite(data.stderr) && (
          <Box sx={{ display: "flex", justifyContent: "space-between", gap: 2 }}>
            <Typography variant="caption" color="text.secondary">
              Std. Error:
            </Typography>
            <Typography variant="caption" sx={{ fontWeight: 600, fontFamily: "monospace" }}>
              {formatScientific(data.stderr, 6)}
            </Typography>
          </Box>
        )}
      </Box>
    </Paper>
  );
};

const SampleChart = ({
  samples,
  isConnected,
  hasRun,
  target = null,
  title = "Integration Mean Convergence",
  lineColor = "#1976d2",
  bandColor = "#1976d2",
  targetLabel = "target",
  xAxisLabel = "Sample Count",
  yAxisLabel = "Mean",
  sampleLabel = "Samples",
  valueLabel = "Mean",
  showStdErr = true,
  showErrorBand = true,
  showTargetLine = true,
  showTargetSummary = true,
}) => {
  const chartData = useMemo(() => buildChartData(samples), [samples]);
  const lastPoint = chartData.length > 0 ? chartData[chartData.length - 1] : null;
  const currentMean = lastPoint ? lastPoint.mean : 0;
  const scalarTarget = useMemo(() => parseScalarTarget(target), [target]);
  const targetValue = scalarTarget?.value ?? null;
  const currentDeltaToTarget = targetValue != null ? currentMean - targetValue : null;

  const xDomain = useMemo(() => {
    const xMin = chartData[0]?.sampleCount ?? 0;
    const xMax = chartData[chartData.length - 1]?.sampleCount ?? 0;
    return xMin === xMax ? [xMin - 1, xMax + 1] : [xMin, xMax];
  }, [chartData]);

  const yDomain = useMemo(() => {
    const yValues = chartData.flatMap((d) => [d.mean, d.lower, d.upper]);
    const yMin = Math.min(...yValues);
    const yMax = Math.max(...yValues);
    const yAbsMax = Math.max(Math.abs(yMin), Math.abs(yMax));
    const yPadding =
      yMin === yMax ? Math.max(Math.abs(yMin) * 0.005, 1e-14) : Math.max((yMax - yMin) * 0.003, yAbsMax * 5e-8, 1e-14);
    return [yMin - yPadding, yMax + yPadding];
  }, [chartData]);

  if (chartData.length === 0) {
    const message =
      isConnected && hasRun ? "No aggregated results yet. Waiting for updates..." : "Select a run to view data";
    return (
      <Box sx={{ mb: 3 }}>
        <Typography variant="h6" gutterBottom>
          {title}
        </Typography>
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
      </Box>
    );
  }

  return (
    <Box sx={{ mb: 3 }}>
      <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 1 }}>
        <Typography variant="h6" sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <TrendingUpIcon />
          {title}
        </Typography>
        <Typography variant="h5" sx={{ fontWeight: 700, fontFamily: "monospace", color: "primary.main" }}>
          {formatScientific(currentMean, 6)}
        </Typography>
      </Box>
      {showTargetSummary && targetValue != null && (
        <Typography variant="body2" sx={{ mb: 1, fontFamily: "monospace" }} color="text.secondary">
          target {formatScientific(targetValue, 6)} | delta {formatScientific(currentDeltaToTarget, 6)}
        </Typography>
      )}

      <Card>
        <CardContent>
          <ResponsiveContainer width="100%" height={500}>
            <ComposedChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 25 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#e0e0e0" />
              <XAxis
                dataKey="sampleCount"
                type="number"
                domain={xDomain}
                label={{ value: xAxisLabel, position: "insideBottom", offset: -10 }}
                stroke="#666"
              />
              <YAxis
                domain={yDomain}
                allowDataOverflow
                tickFormatter={(value) => formatScientific(value, 2, "")}
                tickCount={6}
                label={{ value: yAxisLabel, angle: -90, position: "insideLeft" }}
                stroke="#666"
              />
              <Tooltip
                content={<ScalarTooltip sampleLabel={sampleLabel} valueLabel={valueLabel} showStdErr={showStdErr} />}
              />
              {showTargetLine && targetValue != null && (
                <ReferenceLine
                  y={targetValue}
                  stroke="#2e7d32"
                  strokeWidth={1.5}
                  strokeDasharray="6 4"
                  ifOverflow="hidden"
                  label={{ value: targetLabel, position: "insideTopRight", fill: "#2e7d32" }}
                />
              )}
              {showErrorBand && (
                <Area
                  type="monotone"
                  dataKey="lower"
                  stackId="error-band"
                  stroke="none"
                  fill="transparent"
                  isAnimationActive={false}
                />
              )}
              {showErrorBand && (
                <Area
                  type="monotone"
                  dataKey="spread"
                  stackId="error-band"
                  stroke="none"
                  fill={bandColor}
                  fillOpacity={0.18}
                  isAnimationActive={false}
                />
              )}
              <Line
                type="monotone"
                dataKey="mean"
                stroke={lineColor}
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
            </ComposedChart>
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

export default memo(SampleChart);
