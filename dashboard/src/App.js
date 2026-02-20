import { useState } from "react";
import { Container, Box, Typography, Button, Collapse, Paper, Grid } from "@mui/material";
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
  const observableImplementation = currentRun?.integration_params?.observable_implementation ?? "unknown";
  const [showJson, setShowJson] = useState(false);

  const jsonPanels = [
    { title: "Run Progress (/runs/:id)", data: currentRun ?? null },
    {
      title: "Integration Params (run.integration_params)",
      data: currentRun?.integration_params ?? null,
    },
    { title: "Work Queue Stats (/runs/:id/stats)", data: workQueueStats ?? [] },
    {
      title: "Latest Aggregated Result (/runs/:id/aggregated/latest)",
      data: latestAggregated ?? null,
    },
    {
      title: "Latest Aggregated Observable",
      data: latestAggregated?.aggregated_observable ?? null,
    },
    { title: "Aggregated History (/runs/:id/aggregated)", data: history ?? [] },
  ];

  const derivedSamples = history
    .slice()
    .reverse()
    .map((item) => {
      const observable = item.aggregated_observable || {};
      const sampleCount = observable.count ?? observable.nr_samples ?? 0;
      const mean =
        observable.mean ?? (typeof observable.sum === "number" && sampleCount > 0 ? observable.sum / sampleCount : 0);

      return {
        sampleCount,
        mean,
        value: mean,
      };
    });

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
      <Box sx={{ mb: 3 }}>
        <Button variant="outlined" onClick={() => setShowJson((value) => !value)} sx={{ mb: 1.5 }}>
          {showJson ? "Hide JSON" : "Show JSON"}
        </Button>
        <Collapse in={showJson}>
          <Grid container spacing={2}>
            {jsonPanels.map((panel) => (
              <Grid item xs={12} key={panel.title}>
                <Paper sx={{ p: 2 }}>
                  <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
                    {panel.title}
                  </Typography>
                  <Box component="pre" sx={{ m: 0, overflowX: "auto", fontSize: "0.8rem" }}>
                    {JSON.stringify(panel.data, null, 2)}
                  </Box>
                </Paper>
              </Grid>
            ))}
          </Grid>
        </Collapse>
      </Box>
    </Container>
  );
};

export default App;
