import SamplerDetailsCard from "./SamplerDetailsCard";

const NaiveMonteCarloSamplerPanel = ({ samplerParams, pointSpec }) => (
  <SamplerDetailsCard
    fields={[
      { label: "continuous_dims", value: pointSpec?.continuous_dims },
      { label: "discrete_dims", value: pointSpec?.discrete_dims },
      { label: "training_target_samples", value: samplerParams?.training_target_samples },
    ]}
  />
);

export default NaiveMonteCarloSamplerPanel;
