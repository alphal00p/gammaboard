import SamplerDetailsCard from "./SamplerDetailsCard";

const HavanaSamplerPanel = ({ samplerParams, pointSpec }) => (
  <SamplerDetailsCard
    fields={[
      { label: "continuous_dims", value: pointSpec?.continuous_dims },
      { label: "discrete_dims", value: pointSpec?.discrete_dims },
      { label: "bins", value: samplerParams?.bins },
      { label: "min_samples_for_update", value: samplerParams?.min_samples_for_update },
      { label: "samples_for_update", value: samplerParams?.samples_for_update },
      { label: "initial_training_rate", value: samplerParams?.initial_training_rate },
      { label: "final_training_rate", value: samplerParams?.final_training_rate },
    ]}
  />
);

export default HavanaSamplerPanel;
