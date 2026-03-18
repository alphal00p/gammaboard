import { Alert } from "@mui/material";
import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import SamplerCustomPanel from "./sampler/SamplerCustomPanel";
import { splitKindConfig, toConfigObject } from "../utils/config";
import { getCurrentSampleTask, getTaskTargetLabel } from "../utils/tasks";

const fmtInt = (value) => (Number.isFinite(Number(value)) ? Number(value).toLocaleString() : "n/a");

const SamplerAggregatorPanel = ({ run, tasks = [] }) => {
  const currentTask = getCurrentSampleTask(tasks);
  const integrationParams = toConfigObject(run?.integration_params);
  const pointSpec = toConfigObject(run?.point_spec);
  const samplerConfig = currentTask?.task?.sampler_aggregator ?? null;
  if (!samplerConfig) {
    return (
      <EnginePanelLayout
        title="Sampler Stage"
        genericPanel={<Alert severity="info">No active sample task is currently selected.</Alert>}
        customPanel={null}
        jsonTitle="current sample task JSON"
        jsonData={currentTask?.task ?? null}
      />
    );
  }
  const { implementation, params: samplerParams } = splitKindConfig(samplerConfig, "unknown");
  const rawSamplerData = {
    current_task: currentTask?.task ?? null,
    point_spec: integrationParams?.point_spec ?? run?.point_spec ?? null,
  };

  return (
    <EnginePanelLayout
      title="Sampler Stage"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="warning"
          fields={[
            { label: "task_sequence", value: currentTask?.sequence_nr ?? "n/a", md: 3 },
            { label: "task_state", value: currentTask?.state ?? "n/a", md: 3 },
            { label: "target_samples", value: getTaskTargetLabel(currentTask), md: 3 },
            { label: "produced_samples", value: fmtInt(currentTask?.nr_produced_samples), md: 3 },
            { label: "completed_samples", value: fmtInt(currentTask?.nr_completed_samples), md: 3 },
            { label: "continuous_dims", value: pointSpec?.continuous_dims ?? "n/a", md: 3 },
            { label: "discrete_dims", value: pointSpec?.discrete_dims ?? "n/a", md: 3 },
          ]}
        />
      }
      customPanel={
        <SamplerCustomPanel implementation={implementation} samplerParams={samplerParams} pointSpec={pointSpec} />
      }
      jsonTitle="current sample task JSON"
      jsonData={rawSamplerData}
    />
  );
};

export default SamplerAggregatorPanel;
