import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import TestOnlySinPanel from "./custom/TestOnlySinPanel";
import TestOnlySincPanel from "./custom/TestOnlySincPanel";

const EVALUATOR_CUSTOM_PANELS = {
  test_only_sin: TestOnlySinPanel,
  test_only_sinc: TestOnlySincPanel,
};

const EvaluatorCustomPanel = ({ implementation, evaluatorParams }) => {
  const Panel = EVALUATOR_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="evaluator" implementation={implementation} />;
  }
  return <Panel evaluatorParams={evaluatorParams} />;
};

export default EvaluatorCustomPanel;
