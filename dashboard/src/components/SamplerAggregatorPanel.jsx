import { Alert } from "@mui/material";
import EnginePanelLayout from "./common/EnginePanelLayout";
import PanelCollection from "./panels/PanelCollection";
import { toConfigObject } from "../utils/config";

const SamplerAggregatorPanel = ({ run, panelResponse = null }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const samplerConfig = integrationParams?.sampler_aggregator ?? null;

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
  const rawSamplerData = {
    sampler_aggregator: samplerConfig,
    sampler_aggregator_runner_params: integrationParams?.sampler_aggregator_runner_params ?? null,
    point_spec: run?.point_spec ?? null,
  };

  return (
    <EnginePanelLayout
      title="Sampler Aggregator"
      genericPanel={
        <PanelCollection descriptors={panelResponse?.panels || []} currentPanels={panelResponse?.current || []} />
      }
      customPanel={null}
      jsonTitle="sampler aggregator JSON"
      jsonData={rawSamplerData}
    />
  );
};

export default SamplerAggregatorPanel;
