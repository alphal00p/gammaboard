import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import EvaluatorCustomPanel from "./evaluator/EvaluatorCustomPanel";
import { splitKindConfig, toConfigObject } from "../utils/config";

const EvaluatorPanel = ({ run }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const { implementation, params: evaluatorParams } = splitKindConfig(
    integrationParams.evaluator,
    "unknown",
    integrationParams.evaluator_params,
  );
  const runnerParams = toConfigObject(integrationParams.evaluator_runner_params);
  const evaluatorInitMetadata = toConfigObject(run?.evaluator_init_metadata);

  return (
    <EnginePanelLayout
      title="Evaluator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="secondary"
          fields={[
            { label: "min_loop_time_ms", value: runnerParams.min_loop_time_ms ?? "n/a", md: 6 },
            {
              label: "performance_snapshot_interval_ms",
              value: runnerParams.performance_snapshot_interval_ms ?? "n/a",
              md: 6,
            },
          ]}
        />
      }
      customPanel={
        <EvaluatorCustomPanel
          implementation={implementation}
          evaluatorParams={evaluatorParams}
          evaluatorInitMetadata={evaluatorInitMetadata}
        />
      }
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
