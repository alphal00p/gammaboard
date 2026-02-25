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
import { RunHistoryProvider, useRunHistory } from "./context/RunHistoryContext";
import { deriveObservableMetric } from "./viewmodels/observable";

function App() {
  const { runs, selectedRun, setSelectedRun } = useRuns();

  return (
    <RunHistoryProvider runId={selectedRun}>
      <AppContent runs={runs} selectedRun={selectedRun} setSelectedRun={setSelectedRun} />
    </RunHistoryProvider>
  );
}

const AppContent = ({ runs, selectedRun, setSelectedRun }) => {
  const { run, workerLogs, workQueueStats, history, latestAggregated, isConnected, lastUpdate } = useRunHistory();
  const currentRun = run || runs.find((r) => r.run_id === selectedRun);
  const observableImplementation =
    currentRun?.observable_implementation ||
    currentRun?.integration_params?.observable_implementation ||
    latestAggregated?.observable_implementation ||
    "scalar";

  const derivedSamples = history
    .slice()
    .reverse()
    .map((item) => deriveObservableMetric(item.aggregated_observable || {}, observableImplementation));

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <Box sx={{ mb: 3 }}>
        <Typography variant="h3" component="h1" gutterBottom>
          Gammaboard
        </Typography>
        <Typography variant="body2" color="text.secondary">
          Real-time Monte Carlo simulation monitoring
        </Typography>
      </Box>

      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
      <RunInfo run={currentRun} />
      <ObservablePanel
        run={currentRun}
        samples={derivedSamples}
        isConnected={isConnected}
        latestAggregated={latestAggregated}
        observableImplementation={observableImplementation}
      />
      <EvaluatorPanel run={currentRun} />
      <SamplerAggregatorPanel run={currentRun} stats={workQueueStats} />
      <WorkerLogsPanel logs={workerLogs} runId={selectedRun} />
      <WorkersPanel runId={selectedRun} />
    </Container>
  );
};

export default App;
