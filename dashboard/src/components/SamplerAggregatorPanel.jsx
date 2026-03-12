import WorkQueueStats from "./WorkQueueStats";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";
import { splitKindConfig, toConfigObject } from "../utils/config";

const fmtMs = (value) => (Number.isFinite(Number(value)) ? Number(value).toFixed(4) : "n/a");
const fmtRatio = (value) => (Number.isFinite(Number(value)) ? Number(value).toFixed(4) : "n/a");
const fmtInt = (value) => (Number.isFinite(Number(value)) ? Number(value).toLocaleString() : "n/a");
const fmtTargetDelta = (actual, target, format = (value) => value) => {
  if (!Number.isFinite(Number(actual)) || !Number.isFinite(Number(target))) return "n/a";
  return format(Number(actual) - Number(target));
};

const SamplerAggregatorPanel = ({ run, stats, runtimeSummary = null }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const pointSpec = toConfigObject(run?.point_spec);
  const { implementation, params: samplerParams } = splitKindConfig(
    integrationParams.sampler_aggregator,
    "unknown",
    integrationParams.sampler_aggregator_params ?? run?.sampler_aggregator_params,
  );
  const runnerParams = toConfigObject(
    integrationParams.sampler_aggregator_runner_params ?? run?.sampler_aggregator_runner_params,
  );

  const targetBatchEvalMs = runnerParams.target_batch_eval_ms;
  const targetQueueRemaining = runnerParams.target_queue_remaining;
  const rawSamplerData = {
    sampler_aggregator: integrationParams?.sampler_aggregator ?? null,
    sampler_aggregator_runner_params: integrationParams?.sampler_aggregator_runner_params ?? null,
    summary_metrics: {
      target_batch_eval_ms: targetBatchEvalMs ?? null,
      actual_batch_eval_ms: runtimeSummary?.actual_eval_ms_per_batch ?? null,
      batch_eval_delta_ms:
        Number.isFinite(Number(runtimeSummary?.actual_eval_ms_per_batch)) && Number.isFinite(Number(targetBatchEvalMs))
          ? Number(runtimeSummary.actual_eval_ms_per_batch) - Number(targetBatchEvalMs)
          : null,
      target_queue_remaining: Number.isFinite(Number(targetQueueRemaining)) ? Number(targetQueueRemaining) : null,
      actual_queue_remaining: runtimeSummary?.actual_queue_remaining_ratio ?? null,
      current_batch_size: runtimeSummary?.current_batch_size ?? null,
      eval_ms_per_sample: runtimeSummary?.actual_eval_ms_per_sample ?? null,
      produce_ms_per_sample: runtimeSummary?.produce_ms_per_sample ?? null,
      ingest_ms_per_sample: runtimeSummary?.ingest_ms_per_sample ?? null,
      produced_batches: runtimeSummary?.produced_batches ?? null,
      ingested_batches: runtimeSummary?.ingested_batches ?? null,
      produced_samples: runtimeSummary?.produced_samples ?? null,
      ingested_samples: runtimeSummary?.ingested_samples ?? null,
    },
  };

  return (
    <EnginePanelLayout
      title="Sampler Aggregator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="warning"
          fields={[
            {
              label: "target_batch_eval_ms",
              value: targetBatchEvalMs ?? "n/a",
            },
            {
              label: "actual_batch_eval_ms",
              value: fmtMs(runtimeSummary?.actual_eval_ms_per_batch),
            },
            {
              label: "batch_eval_delta_ms",
              value: fmtTargetDelta(runtimeSummary?.actual_eval_ms_per_batch, targetBatchEvalMs, fmtMs),
            },
            {
              label: "target_queue_remaining",
              value: fmtRatio(targetQueueRemaining),
            },
            {
              label: "actual_queue_remaining",
              value: fmtRatio(runtimeSummary?.actual_queue_remaining_ratio),
            },
            {
              label: "current_batch_size",
              value: fmtInt(runtimeSummary?.current_batch_size),
            },
            {
              label: "eval_ms_per_sample",
              value: fmtMs(runtimeSummary?.actual_eval_ms_per_sample),
            },
            {
              label: "produce_ms_per_sample",
              value: fmtMs(runtimeSummary?.produce_ms_per_sample),
            },
            {
              label: "ingest_ms_per_sample",
              value: fmtMs(runtimeSummary?.ingest_ms_per_sample),
            },
            {
              label: "produced_batches",
              value: fmtInt(runtimeSummary?.produced_batches),
            },
            {
              label: "ingested_batches",
              value: fmtInt(runtimeSummary?.ingested_batches),
            },
            {
              label: "produced_samples",
              value: fmtInt(runtimeSummary?.produced_samples),
            },
            {
              label: "ingested_samples",
              value: fmtInt(runtimeSummary?.ingested_samples),
            },
          ]}
          footer={<WorkQueueStats stats={stats} />}
        />
      }
      customPanel={
        <SamplerCustomPanel implementation={implementation} samplerParams={samplerParams} pointSpec={pointSpec} />
      }
      jsonTitle="sampler_aggregator JSON"
      jsonData={rawSamplerData}
    />
  );
};

export default SamplerAggregatorPanel;
