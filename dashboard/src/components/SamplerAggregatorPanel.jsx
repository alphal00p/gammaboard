import WorkQueueStats from "./WorkQueueStats";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";
import { splitKindConfig, toConfigObject } from "../utils/config";

const fmtMs = (value) => (Number.isFinite(Number(value)) ? Number(value).toFixed(4) : "n/a");
const fmtRatio = (value) => (Number.isFinite(Number(value)) ? Number(value).toFixed(4) : "n/a");

const SamplerAggregatorPanel = ({ run, stats, runtimeSummary = null }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const { implementation, params: samplerParams } = splitKindConfig(
    integrationParams.sampler_aggregator,
    "unknown",
    integrationParams.sampler_aggregator_params ?? run?.sampler_aggregator_params,
  );
  const runnerParams = toConfigObject(
    integrationParams.sampler_aggregator_runner_params ?? run?.sampler_aggregator_runner_params,
  );

  const minPollMs = runnerParams.min_poll_time_ms;
  const maxBatchSize = runnerParams.max_batch_size;
  const maxBatchesPerTick = runnerParams.max_batches_per_tick;
  const targetBatchEvalMs = runnerParams.target_batch_eval_ms;
  const maxQueueSize = runnerParams.max_queue_size;
  const completedBatchFetchLimit = runnerParams.completed_batch_fetch_limit;

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
            {
              label: "avg_queue_depletion",
              value: fmtRatio(runtimeSummary?.avg_queue_depletion),
            },
            {
              label: "avg_time_per_sample_ms",
              value: fmtMs(runtimeSummary?.avg_time_per_sample_ms),
            },
            {
              label: "avg_time_per_batch_ms",
              value: fmtMs(runtimeSummary?.avg_time_per_batch_ms),
            },
          ]}
          footer={<WorkQueueStats stats={stats} />}
        />
      }
      customPanel={<SamplerCustomPanel implementation={implementation} samplerParams={samplerParams} />}
      jsonTitle="sampler_aggregator JSON"
      jsonData={{
        sampler_aggregator: integrationParams?.sampler_aggregator ?? null,
        sampler_aggregator_runner_params: integrationParams?.sampler_aggregator_runner_params ?? null,
      }}
    />
  );
};

export default SamplerAggregatorPanel;
