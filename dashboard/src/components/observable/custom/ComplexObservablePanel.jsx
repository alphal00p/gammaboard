import { Box } from "@mui/material";
import { useMemo } from "react";
import SampleChart from "../../SampleChart";

const mapComplexSeries = (samples, valueKey, stderrKey, lowerKey, upperKey, spreadKey) =>
  (Array.isArray(samples) ? samples : []).map((sample) => ({
    sampleCount: sample.sampleCount,
    mean: sample[valueKey],
    value: sample[valueKey],
    stderr: sample[stderrKey],
    lower: sample[lowerKey],
    upper: sample[upperKey],
    spread: sample[spreadKey],
  }));

const toInverseNormalizedAbsErrorSamples = (samples) =>
  (Array.isArray(samples) ? samples : [])
    .map((sample) => {
      const sampleCount = Number(sample.sampleCount);
      const meanAbs = Number(sample.abs_mean);
      const stderrAbs = Number(sample.abs_stderr);
      if (!Number.isFinite(sampleCount) || sampleCount <= 0) return null;
      if (!Number.isFinite(meanAbs) || meanAbs <= 0) return null;
      if (!Number.isFinite(stderrAbs) || stderrAbs <= 0) return null;
      const value = (meanAbs * meanAbs) / (stderrAbs * stderrAbs);
      if (!Number.isFinite(value)) return null;
      return {
        sampleCount,
        mean: value,
      };
    })
    .filter(Boolean);

const ComplexObservablePanel = ({ samples, isConnected, hasRun }) => {
  const realSamples = useMemo(
    () => mapComplexSeries(samples, "real", "real_stderr", "real_lower", "real_upper", "real_spread"),
    [samples],
  );
  const imagSamples = useMemo(
    () => mapComplexSeries(samples, "imag", "imag_stderr", "imag_lower", "imag_upper", "imag_spread"),
    [samples],
  );
  const inverseNormalizedAbsErrorSamples = useMemo(() => toInverseNormalizedAbsErrorSamples(samples), [samples]);

  return (
    <Box>
      <SampleChart
        samples={realSamples}
        isConnected={isConnected}
        hasRun={hasRun}
        title="Real part vs nr_samples"
        lineColor="#1976d2"
        bandColor="#1976d2"
        xAxisLabel="nr_samples"
        yAxisLabel="real(mean)"
        sampleLabel="nr_samples"
        valueLabel="real(mean)"
        showErrorBand
      />
      <SampleChart
        samples={imagSamples}
        isConnected={isConnected}
        hasRun={hasRun}
        title="Imaginary part vs nr_samples"
        lineColor="#ef6c00"
        bandColor="#ef6c00"
        xAxisLabel="nr_samples"
        yAxisLabel="imag(mean)"
        sampleLabel="nr_samples"
        valueLabel="imag(mean)"
        showErrorBand
      />
      <SampleChart
        samples={inverseNormalizedAbsErrorSamples}
        isConnected={isConnected}
        hasRun={hasRun}
        title="mean(|z|)^2 / err^2 vs nr_samples"
        lineColor="#2e7d32"
        bandColor="#2e7d32"
        xAxisLabel="nr_samples"
        yAxisLabel="mean(|z|)^2 / err^2"
        sampleLabel="nr_samples"
        valueLabel="mean(|z|)^2 / err^2"
        showStdErr={false}
        showErrorBand={false}
        showTargetLine={false}
        showTargetSummary={false}
      />
    </Box>
  );
};

export default ComplexObservablePanel;
