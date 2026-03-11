import EvaluatorDetailsCard from "./EvaluatorDetailsCard";

const SinEvaluatorPanel = ({ evaluatorParams, pointSpec }) => (
  <EvaluatorDetailsCard
    minEvalTimePerSampleMs={evaluatorParams?.min_eval_time_per_sample_ms}
    pointSpec={pointSpec}
    observableKind="scalar"
    integralLatex={String.raw`\int_0^1 dx \,\sin(x)`}
  />
);

export default SinEvaluatorPanel;
