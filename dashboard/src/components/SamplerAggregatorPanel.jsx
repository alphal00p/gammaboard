import WorkQueueStats from "./WorkQueueStats";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";

const SamplerAggregatorPanel = ({ run, stats }) => {
  const integrationParams = run?.integration_params || {};
  const implementation = integrationParams.sampler_aggregator_implementation || "unknown";
  const samplerParams = integrationParams.sampler_aggregator_params || {};
  const runnerParams = integrationParams.sampler_aggregator_runner_params || {};
  const completionRate = run?.completion_rate;

  return (
    <EnginePanelLayout
      title="Sampler Aggregator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="warning"
          fields={[
            { label: "interval_ms", value: runnerParams.interval_ms ?? "n/a" },
            { label: "lease_ttl_ms", value: runnerParams.lease_ttl_ms ?? "n/a" },
            { label: "nr_samples", value: runnerParams.nr_samples ?? "n/a" },
          ]}
          footer={<WorkQueueStats stats={stats} completionRate={completionRate} />}
        />
      }
      customPanel={<SamplerCustomPanel implementation={implementation} samplerParams={samplerParams} />}
      jsonTitle="sampler_aggregator JSON"
      jsonData={{
        sampler_aggregator_params: samplerParams,
        sampler_aggregator_runner_params: runnerParams,
      }}
    />
  );
};

export default SamplerAggregatorPanel;
