import { useMemo } from "react";
import SampleChart from "../../SampleChart";

const toInverseNormalizedErrorSamples = (samples) =>
  (Array.isArray(samples) ? samples : [])
    .map((sample) => {
      const sampleCount = Number(sample.sampleCount);
      const meanAbs = Number(sample.meanAbs);
      const stderr = Number(sample.stderr);
      if (!Number.isFinite(sampleCount) || sampleCount <= 0) return null;
      if (!Number.isFinite(meanAbs) || meanAbs <= 0) return null;
      if (!Number.isFinite(stderr) || stderr <= 0) return null;

      // Relative normalized error: stderr / mean(abs)
      const normalizedError = stderr / meanAbs;
      if (!Number.isFinite(normalizedError) || normalizedError === 0) return null;

      const inverseAbsNormalizedError = Math.abs(1 / normalizedError);
      if (!Number.isFinite(inverseAbsNormalizedError)) return null;

      return {
        sampleCount: Math.sqrt(sampleCount),
        mean: inverseAbsNormalizedError,
      };
    })
    .filter(Boolean);

const ScalarObservablePanel = ({ samples, isConnected, hasRun, target }) => {
  const inverseNormalizedErrorSamples = useMemo(() => toInverseNormalizedErrorSamples(samples), [samples]);

  return (
    <>
      <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} target={target} />
      {inverseNormalizedErrorSamples.length > 0 && (
        <SampleChart
          samples={inverseNormalizedErrorSamples}
          isConnected={isConnected}
          hasRun={hasRun}
          title="mean(abs) / stderr vs sqrt(Sample Count)"
          lineColor="#2e7d32"
          bandColor="#2e7d32"
          xAxisLabel="sqrt(Sample Count)"
          yAxisLabel="mean(abs) / stderr"
          sampleLabel="sqrt(N)"
          valueLabel="mean(abs)/stderr"
          showStdErr={false}
          showErrorBand={false}
          showTargetLine={false}
          showTargetSummary={false}
        />
      )}
    </>
  );
};

export default ScalarObservablePanel;
