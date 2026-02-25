import EnginePanelLayout from "./common/EnginePanelLayout";
import ImplementationSummaryCard from "./common/ImplementationSummaryCard";
import EvaluatorCustomPanel from "./evaluator/EvaluatorCustomPanel";

const EvaluatorPanel = ({ run }) => {
  const integrationParams = run?.integration_params || {};
  const implementation = integrationParams.evaluator_implementation || "unknown";
  const evaluatorParams = integrationParams.evaluator_params || {};
  const runnerParams = integrationParams.evaluator_runner_params || {};

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
      customPanel={<EvaluatorCustomPanel implementation={implementation} evaluatorParams={evaluatorParams} />}
      jsonTitle="evaluator JSON"
      jsonData={{
        evaluator_params: evaluatorParams,
        evaluator_runner_params: runnerParams,
      }}
    />
  );
};

export default EvaluatorPanel;
