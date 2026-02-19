import { Container, Box, Typography } from "@mui/material";
import ConnectionStatus from "./components/ConnectionStatus";
import RunSelector from "./components/RunSelector";
import RunInfo from "./components/RunInfo";
import WorkQueueStats from "./components/WorkQueueStats";
import AggregatedBatchesPanel from "./components/AggregatedBatchesPanel";
import SampleChart from "./components/SampleChart";
import { useRuns } from "./hooks/useRuns";
import { RunHistoryProvider, useRunHistory } from "./context/RunHistoryContext";

function App() {
  const { runs, selectedRun, setSelectedRun } = useRuns();

  return (
    <RunHistoryProvider runId={selectedRun}>
      <AppContent runs={runs} selectedRun={selectedRun} setSelectedRun={setSelectedRun} />
    </RunHistoryProvider>
  );
}

const AppContent = ({ runs, selectedRun, setSelectedRun }) => {
  const { run, workQueueStats, history, latestAggregated, isConnected, lastUpdate } = useRunHistory();
  const currentRun = run || runs.find((r) => r.run_id === selectedRun);
  const observableImplementation = currentRun?.integration_params?.observable_implementation || null;

  const derivedSamples = history
    .slice()
    .reverse()
    .map((item) => ({
      sampleCount: item.aggregated_observable?.nr_samples ?? 0,
      mean: item.aggregated_observable?.mean ?? 0,
      value: item.aggregated_observable?.mean ?? 0,
    }));

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
      <WorkQueueStats stats={workQueueStats} completionRate={currentRun?.completion_rate} />
      <AggregatedBatchesPanel latestAggregated={latestAggregated} run={currentRun} />
      <SampleChart
        samples={derivedSamples}
        isConnected={isConnected}
        currentRun={currentRun}
        latestAggregated={latestAggregated}
        observableImplementation={observableImplementation}
      />
    </Container>
  );
};

export default App;
