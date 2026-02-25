import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import HavanaSamplerPanel from "./custom/HavanaSamplerPanel";
import TestOnlyTrainingSamplerPanel from "./custom/TestOnlyTrainingSamplerPanel";

const SAMPLER_CUSTOM_PANELS = {
  havana: HavanaSamplerPanel,
  test_only_training: TestOnlyTrainingSamplerPanel,
};

const SamplerCustomPanel = ({ implementation, samplerParams }) => {
  const Panel = SAMPLER_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="sampler_aggregator" implementation={implementation} />;
  }
  return <Panel samplerParams={samplerParams} />;
};

export default SamplerCustomPanel;
