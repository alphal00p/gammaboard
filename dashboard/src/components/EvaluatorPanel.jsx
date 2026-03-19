import EnginePanelLayout from "./common/EnginePanelLayout";
import PanelCollection from "./panels/PanelCollection";
import { toConfigObject } from "../utils/config";

const EvaluatorPanel = ({ run, panelResponse = null }) => {
  const integrationParams = toConfigObject(run?.integration_params);

  return (
    <EnginePanelLayout
      title="Evaluator"
      genericPanel={
        <PanelCollection descriptors={panelResponse?.panels || []} currentPanels={panelResponse?.current || []} />
      }
      customPanel={null}
      jsonTitle="evaluator JSON"
      jsonData={{
        evaluator: integrationParams?.evaluator ?? null,
        evaluator_runner_params: integrationParams?.evaluator_runner_params ?? null,
        evaluator_init_metadata: run?.evaluator_init_metadata ?? null,
      }}
    />
  );
};

export default EvaluatorPanel;
