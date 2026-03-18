import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import HavanaSamplerPanel from "./custom/HavanaSamplerPanel";
import NaiveMonteCarloSamplerPanel from "./custom/NaiveMonteCarloSamplerPanel";

const SAMPLER_CUSTOM_PANELS = {
  havana_training: HavanaSamplerPanel,
  havana_inference: HavanaSamplerPanel,
  naive_monte_carlo: NaiveMonteCarloSamplerPanel,
};

const SamplerCustomPanel = ({ implementation, samplerParams, pointSpec }) => {
  const Panel = SAMPLER_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="sampler_aggregator" implementation={implementation} />;
  }
  return <Panel samplerParams={samplerParams} pointSpec={pointSpec} />;
};

export default SamplerCustomPanel;
