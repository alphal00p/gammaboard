import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import SymbolicaPanel from "./custom/SymbolicaPanel";
import SinEvaluatorPanel from "./custom/SinEvaluatorPanel";
import SincEvaluatorPanel from "./custom/SincEvaluatorPanel";

const EVALUATOR_CUSTOM_PANELS = {
  symbolica: SymbolicaPanel,
  sin_evaluator: SinEvaluatorPanel,
  sinc_evaluator: SincEvaluatorPanel,
};

const EvaluatorCustomPanel = ({ implementation, evaluatorParams, evaluatorInitMetadata, pointSpec }) => {
  const Panel = EVALUATOR_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="evaluator" implementation={implementation} />;
  }
  return (
    <Panel evaluatorParams={evaluatorParams} evaluatorInitMetadata={evaluatorInitMetadata} pointSpec={pointSpec} />
  );
};

export default EvaluatorCustomPanel;
