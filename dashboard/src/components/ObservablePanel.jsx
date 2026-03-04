import { Typography } from "@mui/material";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import ObservableCustomPanel from "./observable/ObservableCustomPanel";
import { formatDateTime, formatScientific, toFiniteNumber } from "../utils/formatters";

const computeScalarRsd = (observable) => {
  const count = toFiniteNumber(observable?.count, 0);
  if (count <= 0) return null;

  const sumWeight = toFiniteNumber(observable?.sum_weight, 0);
  const sumSq = toFiniteNumber(observable?.sum_sq, 0);
  const sumAbs = toFiniteNumber(observable?.sum_abs, 0);
  const mean = sumWeight / count;
  const meanAbs = sumAbs / count;
  if (!Number.isFinite(meanAbs) || meanAbs <= 0) return null;

  const variance = Math.max(0, sumSq / count - mean * mean);
  const std = Math.sqrt(variance);
  const rsd = std / meanAbs;
  return Number.isFinite(rsd) ? rsd : null;
};

const computeComplexRsd = (observable) => {
  const count = toFiniteNumber(observable?.count, 0);
  if (count <= 0) return null;

  const absSum = toFiniteNumber(observable?.abs_sum, 0);
  const absSqSum = toFiniteNumber(observable?.abs_sq_sum, 0);
  const meanAbs = absSum / count;
  if (!Number.isFinite(meanAbs) || meanAbs <= 0) return null;

  const varianceAbs = Math.max(0, absSqSum / count - meanAbs * meanAbs);
  const stdAbs = Math.sqrt(varianceAbs);
  const rsd = stdAbs / meanAbs;
  return Number.isFinite(rsd) ? rsd : null;
};

const computeRsd = (observable, implementation) => {
  if (!observable || typeof observable !== "object") return null;
  if (implementation === "complex") return computeComplexRsd(observable);
  return computeScalarRsd(observable);
};

const ObservablePanel = ({ run, latestAggregated, samples, totalSnapshots, isConnected, observableImplementation }) => {
  const integrationParams = run?.integration_params || {};
  const observableParams = integrationParams.observable_params || {};
  const observablePayload = latestAggregated?.aggregated_observable || null;

  const aggregatedBatches = toFiniteNumber(
    observablePayload?.nr_batches ?? run?.batches_completed ?? run?.completed_batches ?? 0,
    0,
  );
  const aggregatedSamples = toFiniteNumber(observablePayload?.count ?? observablePayload?.nr_samples, 0);
  const rsd = computeRsd(observablePayload, observableImplementation);
  const historySnapshots = Array.isArray(samples) ? samples.length : 0;
  const fullHistorySnapshots = Number.isFinite(totalSnapshots) ? totalSnapshots : historySnapshots;

  return (
    <EnginePanelLayout
      title="Observable"
      genericPanel={
        <ImplementationSummaryCard
          implementation={observableImplementation}
          chipColor="primary"
          fields={[
            {
              label: "history snapshots",
              value: `${historySnapshots.toLocaleString()} / ${fullHistorySnapshots.toLocaleString()}`,
            },
            { label: "aggregated batches", value: aggregatedBatches.toLocaleString() },
            { label: "aggregated samples", value: aggregatedSamples.toLocaleString() },
            { label: "RSD (std/mean(abs))", value: rsd == null ? "n/a" : formatScientific(rsd, 4) },
          ]}
          footer={
            <>
              <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
                Latest Snapshot
              </Typography>
              <Typography variant="caption" color="text.secondary">
                id: {latestAggregated?.id ?? "n/a"} | created_at: {formatDateTime(latestAggregated?.created_at)}
              </Typography>
            </>
          }
        />
      }
      customPanel={
        <ObservableCustomPanel
          implementation={observableImplementation}
          samples={samples}
          isConnected={isConnected}
          hasRun={Boolean(run)}
          target={run?.target}
        />
      }
      jsonTitle="observable JSON"
      jsonData={{
        observable_params: observableParams,
        aggregated_observable: observablePayload,
      }}
    />
  );
};

export default ObservablePanel;
