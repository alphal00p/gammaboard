import WorkQueueStats from "./WorkQueueStats";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";

const toObject = (value) => {
  if (!value) return {};
  if (typeof value === "object" && !Array.isArray(value)) return value;
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value);
      return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
    } catch {
      return {};
    }
  }
  return {};
};

const SamplerAggregatorPanel = ({ run, stats }) => {
  const integrationParams = toObject(run?.integration_params);
  const implementation = integrationParams.sampler_aggregator_implementation || "unknown";
  const samplerParams = toObject(integrationParams.sampler_aggregator_params ?? run?.sampler_aggregator_params);
  const runnerParams = toObject(
    integrationParams.sampler_aggregator_runner_params ?? run?.sampler_aggregator_runner_params,
  );

  const minPollMs = runnerParams.min_poll_time_ms ?? runnerParams.interval_ms;
  const maxBatchSize = runnerParams.max_batch_size ?? runnerParams.nr_samples;
  const maxBatchesPerTick = runnerParams.max_batches_per_tick ?? runnerParams.max_nr_batches;
  const targetBatchEvalMs = runnerParams.target_batch_eval_ms ?? runnerParams.target_eval_time_ms;
  const maxQueueSize = runnerParams.max_queue_size ?? runnerParams.max_pending_batches;
  const completedBatchFetchLimit = runnerParams.completed_batch_fetch_limit ?? runnerParams.completed_batch_limit;

  return (
    <EnginePanelLayout
      title="Sampler Aggregator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="warning"
          fields={[
            { label: "min_poll_time_ms", value: minPollMs ?? "n/a" },
            { label: "lease_ttl_ms", value: runnerParams.lease_ttl_ms ?? "n/a" },
            { label: "max_batch_size", value: maxBatchSize ?? "n/a" },
            { label: "max_batches_per_tick", value: maxBatchesPerTick ?? "n/a" },
            {
              label: "target_batch_eval_ms",
              value: targetBatchEvalMs ?? "n/a",
            },
            { label: "max_queue_size", value: maxQueueSize ?? "n/a" },
            {
              label: "completed_batch_fetch_limit",
              value: completedBatchFetchLimit ?? "n/a",
            },
          ]}
          footer={<WorkQueueStats stats={stats} />}
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
