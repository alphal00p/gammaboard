import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import EvaluatorCustomPanel from "./evaluator/EvaluatorCustomPanel";
import { deriveObservableImplementation, splitKindConfig, toConfigObject } from "../utils/config";

const EvaluatorPanel = ({ run }) => {
  const integrationParams = toConfigObject(run?.integration_params);
  const pointSpec = toConfigObject(run?.point_spec);
  const { implementation, params: evaluatorParams } = splitKindConfig(
    integrationParams.evaluator,
    "unknown",
    integrationParams.evaluator_params,
  );
  const runnerParams = toConfigObject(integrationParams.evaluator_runner_params);
  const evaluatorInitMetadata = toConfigObject(run?.evaluator_init_metadata);
  const observableKind = deriveObservableImplementation(integrationParams.evaluator, null, "unknown");

  return (
    <EnginePanelLayout
      title="Evaluator"
      genericPanel={
        <ImplementationSummaryCard
          implementation={implementation}
          chipColor="secondary"
          fields={[
            {
              label: "performance_snapshot_interval_ms",
              value: runnerParams.performance_snapshot_interval_ms ?? "n/a",
              md: 6,
            },
            {
              label: "observable_kind",
              value: observableKind,
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
          pointSpec={pointSpec}
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
