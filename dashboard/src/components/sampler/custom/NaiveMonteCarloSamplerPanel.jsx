import SamplerDetailsCard from "./SamplerDetailsCard";

const NaiveMonteCarloSamplerPanel = ({ samplerParams }) => (
  <SamplerDetailsCard
    fields={[
      { label: "continuous_dims", value: samplerParams?.continuous_dims },
      { label: "discrete_dims", value: samplerParams?.discrete_dims },
      { label: "training_target_samples", value: samplerParams?.training_target_samples },
    ]}
  />
);

export default NaiveMonteCarloSamplerPanel;
