import SamplerDetailsCard from "./SamplerDetailsCard";

const HavanaSamplerPanel = ({ samplerParams }) => (
  <SamplerDetailsCard
    fields={[
      { label: "continuous_dims", value: samplerParams?.continuous_dims },
      { label: "bins", value: samplerParams?.bins },
      { label: "min_samples_for_update", value: samplerParams?.min_samples_for_update },
    ]}
  />
);

export default HavanaSamplerPanel;
