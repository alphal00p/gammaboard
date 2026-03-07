import EvaluatorDetailsCard from "./EvaluatorDetailsCard";

const SincEvaluatorPanel = ({ evaluatorParams }) => (
  <EvaluatorDetailsCard
    minEvalTimePerSampleMs={evaluatorParams?.min_eval_time_per_sample_ms}
    expectedContinuousDims={2}
    observableKind="complex"
    integralLatex={String.raw`\int_0^1 dx \int_0^1 dy \,\sin\!\left(x + i y\right)`}
  />
);

export default SincEvaluatorPanel;
