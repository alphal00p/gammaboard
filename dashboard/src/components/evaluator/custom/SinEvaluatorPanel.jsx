import EvaluatorDetailsCard from "./EvaluatorDetailsCard";

const SinEvaluatorPanel = ({ evaluatorParams }) => (
  <EvaluatorDetailsCard
    minEvalTimePerSampleMs={evaluatorParams?.min_eval_time_per_sample_ms}
    expectedContinuousDims={1}
    supportedObservable="scalar"
    integralLatex={String.raw`\int_0^1 dx \,\sin(x)`}
  />
);

export default SinEvaluatorPanel;
