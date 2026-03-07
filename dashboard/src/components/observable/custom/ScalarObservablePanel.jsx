import { useMemo } from "react";
import SampleChart from "../../SampleChart";

const toAbsSignalToNoiseSquaredSamples = (samples) =>
  (Array.isArray(samples) ? samples : [])
    .map((sample) => {
      const sampleCount = Number(sample.sampleCount);
      const meanAbs = Number(sample.meanAbs);
      const stderr = Number(sample.stderr);
      if (!Number.isFinite(sampleCount) || sampleCount <= 0) return null;
      if (!Number.isFinite(meanAbs) || meanAbs <= 0) return null;
      if (!Number.isFinite(stderr) || stderr <= 0) return null;

      const value = (meanAbs * meanAbs) / (stderr * stderr);
      if (!Number.isFinite(value)) return null;

      return {
        sampleCount,
        mean: value,
      };
    })
    .filter(Boolean);

const ScalarObservablePanel = ({ samples, isConnected, hasRun, target }) => {
  const absSignalToNoiseSquaredSamples = useMemo(() => toAbsSignalToNoiseSquaredSamples(samples), [samples]);

  return (
    <>
      <SampleChart samples={samples} isConnected={isConnected} hasRun={hasRun} target={target} />
      {absSignalToNoiseSquaredSamples.length > 0 && (
        <SampleChart
          samples={absSignalToNoiseSquaredSamples}
          isConnected={isConnected}
          hasRun={hasRun}
          title="mean(abs)^2 / err^2 vs nr_samples"
          lineColor="#2e7d32"
          bandColor="#2e7d32"
          xAxisLabel="nr_samples"
          yAxisLabel="mean(abs)^2 / err^2"
          sampleLabel="nr_samples"
          valueLabel="mean(abs)^2 / err^2"
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
