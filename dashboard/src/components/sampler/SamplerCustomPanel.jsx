import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import HavanaSamplerPanel from "./custom/HavanaSamplerPanel";
import NaiveMonteCarloSamplerPanel from "./custom/NaiveMonteCarloSamplerPanel";

const SAMPLER_CUSTOM_PANELS = {
  havana: HavanaSamplerPanel,
  naive_monte_carlo: NaiveMonteCarloSamplerPanel,
};

const SamplerCustomPanel = ({ implementation, samplerParams }) => {
  const Panel = SAMPLER_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="sampler_aggregator" implementation={implementation} />;
  }
  return <Panel samplerParams={samplerParams} />;
};

export default SamplerCustomPanel;
