import EvaluatorDetailsCard from "./EvaluatorDetailsCard";

const SincEvaluatorPanel = ({ evaluatorParams, pointSpec }) => (
  <EvaluatorDetailsCard
    minEvalTimePerSampleMs={evaluatorParams?.min_eval_time_per_sample_ms}
    pointSpec={pointSpec}
    observableKind="complex"
    integralLatex={String.raw`\int_0^1 dx \int_0^1 dy \,\sin\!\left(x + i y\right)`}
  />
);

export default SincEvaluatorPanel;
