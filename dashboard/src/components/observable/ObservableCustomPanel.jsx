import UnsupportedImplementationPanel from "../common/UnsupportedImplementationPanel";
import ComplexObservablePanel from "./custom/ComplexObservablePanel";
import ScalarObservablePanel from "./custom/ScalarObservablePanel";

const OBSERVABLE_CUSTOM_PANELS = {
  scalar: ScalarObservablePanel,
  complex: ComplexObservablePanel,
};

const ObservableCustomPanel = ({ implementation, samples, isConnected, hasRun, target }) => {
  const Panel = OBSERVABLE_CUSTOM_PANELS[implementation];
  if (!Panel) {
    return <UnsupportedImplementationPanel kind="observable" implementation={implementation} />;
  }
  return <Panel samples={samples} isConnected={isConnected} hasRun={hasRun} target={target} />;
};

export default ObservableCustomPanel;
