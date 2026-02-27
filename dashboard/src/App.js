import { Container, Box, Typography } from "@mui/material";
import ConnectionStatus from "./components/ConnectionStatus";
import RunSelector from "./components/RunSelector";
import RunInfo from "./components/RunInfo";
import ObservablePanel from "./components/ObservablePanel";
import EvaluatorPanel from "./components/EvaluatorPanel";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import WorkerLogsPanel from "./components/WorkerLogsPanel";
import WorkersPanel from "./components/WorkersPanel";
import { useRuns } from "./hooks/useRuns";
import { RunHistoryProvider, useRunAggregated, useRunMeta, useRunQueueLogs } from "./context/RunHistoryContext";
import { deriveObservableMetric } from "./viewmodels/observable";
import { useMemo } from "react";

function App() {
  const { runs, selectedRun, setSelectedRun } = useRuns();

  return (
    <RunHistoryProvider runId={selectedRun}>
      <AppContent runs={runs} selectedRun={selectedRun} setSelectedRun={setSelectedRun} />
    </RunHistoryProvider>
  );
}

const useCurrentRun = (runs, selectedRun) => {
  const { run } = useRunMeta();
  return run || runs.find((r) => r.run_id === selectedRun);
};

const ConnectionAndSelector = ({ runs, selectedRun, setSelectedRun }) => {
  const { isConnected, lastUpdate } = useRunMeta();

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
    </>
  );
};

const RunPanels = ({ runs, selectedRun }) => {
  const currentRun = useCurrentRun(runs, selectedRun);

  return (
    <>
      <RunInfo run={currentRun} />
      <EvaluatorPanel run={currentRun} />
    </>
  );
};

const ObservableSection = ({ runs, selectedRun }) => {
  const currentRun = useCurrentRun(runs, selectedRun);
  const { history, latestAggregated } = useRunAggregated();
  const { isConnected } = useRunMeta();
  const observableImplementation =
    currentRun?.observable_implementation ||
    currentRun?.integration_params?.observable_implementation ||
    latestAggregated?.observable_implementation ||
    "scalar";

  const derivedSamples = useMemo(
    () =>
      history
        .slice()
        .reverse()
        .map((item) => deriveObservableMetric(item.aggregated_observable || {}, observableImplementation)),
    [history, observableImplementation],
  );

  return (
    <ObservablePanel
      run={currentRun}
      samples={derivedSamples}
      isConnected={isConnected}
      latestAggregated={latestAggregated}
      observableImplementation={observableImplementation}
    />
  );
};

const SamplerSection = ({ runs, selectedRun }) => {
  const currentRun = useCurrentRun(runs, selectedRun);
  const { workQueueStats } = useRunQueueLogs();

  return <SamplerAggregatorPanel run={currentRun} stats={workQueueStats} />;
};

const WorkerLogsSection = ({ selectedRun }) => {
  const { workerLogs } = useRunQueueLogs();

  return <WorkerLogsPanel logs={workerLogs} runId={selectedRun} />;
};

const AppContent = ({ runs, selectedRun, setSelectedRun }) => (
  <Container maxWidth="xl" sx={{ py: 3 }}>
    <Box sx={{ mb: 3 }}>
      <Typography variant="h3" component="h1" gutterBottom>
        Gammaboard
      </Typography>
      <Typography variant="body2" color="text.secondary">
        Real-time Monte Carlo simulation monitoring
      </Typography>
    </Box>

    <ConnectionAndSelector runs={runs} selectedRun={selectedRun} setSelectedRun={setSelectedRun} />
    <RunPanels runs={runs} selectedRun={selectedRun} />
    <ObservableSection runs={runs} selectedRun={selectedRun} />
    <SamplerSection runs={runs} selectedRun={selectedRun} />
    <WorkerLogsSection selectedRun={selectedRun} />
    <WorkersPanel runId={selectedRun} />
  </Container>
);

export default App;
