import { Alert, Box, Container, Tab, Tabs, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import ConnectionStatus from "./components/ConnectionStatus";
import EvaluatorPanel from "./components/EvaluatorPanel";
import HistoryRangeControls from "./components/HistoryRangeControls";
import ObservablePanel from "./components/ObservablePanel";
import RunInfo from "./components/RunInfo";
import RunSelector from "./components/RunSelector";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import LogsWorkspace from "./components/LogsWorkspace";
import WorkersWorkspace from "./components/WorkersWorkspace";
import RunScopedWorkspace from "./components/common/RunScopedWorkspace";
import { RunHistoryProvider, useRunHistory } from "./context/RunHistoryContext";
import { useSamplerRuntimeSummary } from "./hooks/useSamplerRuntimeSummary";
import { useRuns } from "./hooks/useRuns";
import { deriveObservableImplementation } from "./utils/config";
import { deriveObservableMetric } from "./viewmodels/observable";

const DASHBOARD_HISTORY_CONFIG = {
  historyBufferMax: 100,
  workQueueStatsLimit: 200,
  pollIntervalMs: 5000,
};

const DashboardHeader = () => (
  <Box sx={{ mb: 3 }}>
    <Typography variant="h3" component="h1" gutterBottom>
      Gammaboard
    </Typography>
    <Typography variant="body2" color="text.secondary">
      Real-time Monte Carlo simulation monitoring
    </Typography>
  </Box>
);

const useCurrentRun = (runs, selectedRun) => {
  const { run } = useRunHistory();
  return run || runs.find((entry) => entry.run_id === selectedRun);
};

const RunModeContent = ({ runs, selectedRun, setSelectedRun, historyRange, setHistoryRange }) => {
  const { isConnected, lastUpdate, history, latestAggregated, workQueueStats } = useRunHistory();
  const currentRun = useCurrentRun(runs, selectedRun);
  const samplerRuntimeSummary = useSamplerRuntimeSummary(selectedRun, 3000);
  const observableImplementation = deriveObservableImplementation(
    currentRun?.integration_params?.evaluator,
    latestAggregated?.aggregated_observable,
    "scalar",
  );

  const fullSamples = useMemo(
    () =>
      history
        .slice()
        .reverse()
        .map((item) => deriveObservableMetric(item.aggregated_observable || {}, observableImplementation)),
    [history, observableImplementation],
  );

  if (!currentRun) {
    return (
      <Alert severity="warning" sx={{ mb: 3 }}>
        Selected run not found in current run list.
      </Alert>
    );
  }

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
      <HistoryRangeControls historyRange={historyRange} setHistoryRange={setHistoryRange} />
      <RunInfo run={currentRun} />
      <EvaluatorPanel run={currentRun} />
      <ObservablePanel
        run={currentRun}
        samples={fullSamples}
        isConnected={isConnected}
        latestAggregated={latestAggregated}
        observableImplementation={observableImplementation}
      />
      <SamplerAggregatorPanel run={currentRun} stats={workQueueStats} runtimeSummary={samplerRuntimeSummary} />
    </>
  );
};

const RunsWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected, historyRange, setHistoryRange }) => {
  return (
    <RunScopedWorkspace
      runs={runs}
      selectedRun={selectedRun}
      setSelectedRun={setSelectedRun}
      isConnected={isConnected}
      noRunsMessage="Create a run to start monitoring history, observables, and engine configuration."
      noSelectionMessage="Pick a run to view run-level metrics and configuration."
    >
      <RunHistoryProvider
        runId={selectedRun}
        historyStart={historyRange.start}
        historyStop={historyRange.end}
        {...DASHBOARD_HISTORY_CONFIG}
      >
        <RunModeContent
          runs={runs}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          historyRange={historyRange}
          setHistoryRange={setHistoryRange}
        />
      </RunHistoryProvider>
    </RunScopedWorkspace>
  );
};

function App() {
  const { runs, isConnected } = useRuns();
  const [mode, setMode] = useState("runs");
  const [historyRange, setHistoryRange] = useState({ start: -50, end: -1 });
  const [selectedRun, setSelectedRun] = useState(null);
  const [selectedLogRun, setSelectedLogRun] = useState(null);

  useEffect(() => {
    if (!Array.isArray(runs) || runs.length === 0) {
      setSelectedRun(null);
      setSelectedLogRun(null);
      return;
    }

    if (!selectedRun || !runs.some((run) => run.run_id === selectedRun)) {
      setSelectedRun(runs[0].run_id);
    }

    if (!selectedLogRun || !runs.some((run) => run.run_id === selectedLogRun)) {
      setSelectedLogRun(runs[0].run_id);
    }
  }, [runs, selectedRun, selectedLogRun]);

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <DashboardHeader />

      <Tabs value={mode} onChange={(_, next) => setMode(next)} sx={{ mb: 3 }}>
        <Tab value="runs" label="Runs" />
        <Tab value="workers" label="Workers" />
        <Tab value="logs" label="Logs" />
      </Tabs>

      {mode === "runs" ? (
        <RunsWorkspace
          runs={runs}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
          historyRange={historyRange}
          setHistoryRange={setHistoryRange}
        />
      ) : mode === "workers" ? (
        <WorkersWorkspace />
      ) : (
        <LogsWorkspace
          runs={runs}
          selectedRun={selectedLogRun}
          setSelectedRun={setSelectedLogRun}
          isConnected={isConnected}
        />
      )}
    </Container>
  );
}

export default App;
