import { Typography } from "@mui/material";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import ObservableCustomPanel from "./observable/ObservableCustomPanel";
import { formatDateTime, toFiniteNumber } from "../utils/formatters";

const ObservablePanel = ({ run, latestAggregated, samples, isConnected, observableImplementation }) => {
  const integrationParams = run?.integration_params || {};
  const observableParams = integrationParams.observable_params || {};
  const observablePayload = latestAggregated?.aggregated_observable || null;

  const aggregatedBatches = toFiniteNumber(
    observablePayload?.nr_batches ?? run?.batches_completed ?? run?.completed_batches ?? 0,
    0,
  );
  const aggregatedSamples = toFiniteNumber(observablePayload?.count ?? observablePayload?.nr_samples, 0);
  const historySnapshots = Array.isArray(samples) ? samples.length : 0;

  return (
    <EnginePanelLayout
      title="Observable"
      genericPanel={
        <ImplementationSummaryCard
          implementation={observableImplementation}
          chipColor="primary"
          fields={[
            { label: "history snapshots", value: historySnapshots.toLocaleString() },
            { label: "aggregated batches", value: aggregatedBatches.toLocaleString() },
            { label: "aggregated samples", value: aggregatedSamples.toLocaleString() },
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
