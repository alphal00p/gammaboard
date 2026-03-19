import { Alert } from "@mui/material";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";
import { splitKindConfig, toConfigObject } from "../utils/config";

const fmtNumber = (value, fallback = "n/a") =>
  Number.isFinite(Number(value)) ? Number(value).toLocaleString() : fallback;

const SamplerAggregatorPanel = ({ run }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const pointSpec = toConfigObject(run?.point_spec);
  const samplerConfig = integrationParams?.sampler_aggregator ?? null;
  const runnerParams = toConfigObject(integrationParams?.sampler_aggregator_runner_params);

  if (!samplerConfig) {
    return (
      <EnginePanelLayout
        title="Sampler Aggregator"
        genericPanel={<Alert severity="info">No sampler aggregator is configured for this run.</Alert>}
        customPanel={null}
        jsonTitle="sampler aggregator JSON"
        jsonData={null}
      />
    );
  }

  const { implementation, params: samplerParams } = splitKindConfig(samplerConfig, "unknown");
  const rawSamplerData = {
    sampler_aggregator: samplerConfig,
    sampler_aggregator_runner_params: integrationParams?.sampler_aggregator_runner_params ?? null,
    point_spec: run?.point_spec ?? null,
  };

  return (
    <EnginePanelLayout
      title="Sampler Aggregator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="warning"
          fields={[
            { label: "continuous_dims", value: pointSpec?.continuous_dims ?? "n/a", md: 3 },
            { label: "discrete_dims", value: pointSpec?.discrete_dims ?? "n/a", md: 3 },
            { label: "target_queue_remaining", value: runnerParams?.target_queue_remaining ?? "n/a", md: 3 },
            { label: "max_batch_size", value: fmtNumber(runnerParams?.max_batch_size), md: 3 },
            { label: "max_queue_size", value: fmtNumber(runnerParams?.max_queue_size), md: 3 },
            { label: "max_batches_per_tick", value: fmtNumber(runnerParams?.max_batches_per_tick), md: 3 },
            { label: "completed_fetch_limit", value: fmtNumber(runnerParams?.completed_batch_fetch_limit), md: 3 },
            { label: "snapshot_interval_ms", value: fmtNumber(runnerParams?.performance_snapshot_interval_ms), md: 3 },
          ]}
        />
      }
      customPanel={
        <SamplerCustomPanel
          implementation={implementation}
          samplerParams={samplerParams}
          pointSpec={pointSpec}
        />
      }
      jsonTitle="sampler aggregator JSON"
      jsonData={rawSamplerData}
    />
  );
};

export default SamplerAggregatorPanel;
