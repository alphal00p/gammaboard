import { Container, Box, Typography, Stack, TextField } from "@mui/material";
import ConnectionStatus from "./components/ConnectionStatus";
import RunSelector from "./components/RunSelector";
import RunInfo from "./components/RunInfo";
import ObservablePanel from "./components/ObservablePanel";
import EvaluatorPanel from "./components/EvaluatorPanel";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import WorkerLogsPanel from "./components/WorkerLogsPanel";
import WorkersPanel from "./components/WorkersPanel";
import { useRuns } from "./hooks/useRuns";
import {
  RunHistoryProvider,
  useRunAggregated,
  useRunConnection,
  useRunHeartbeat,
  useRunQueueLogs,
  useRunState,
} from "./context/RunHistoryContext";
import { deriveObservableMetric } from "./viewmodels/observable";
import { Profiler, useMemo, useState } from "react";

const DASHBOARD_HISTORY_CONFIG = {
  historyLimit: 200,
  historyBufferMax: 500,
  workerLogsLimit: 200,
  workQueueStatsLimit: 200,
  pollIntervalMs: 5000,
  sseConnectedPollThrottleFactor: 4,
  streamIntervalMs: 1000,
};

const onRender = (id, phase, actualDuration, baseDuration, startTime, commitTime) => {
  if (process.env.NODE_ENV !== "development") return;
  console.debug("[react-profiler]", {
    id,
    phase,
    actualDuration,
    baseDuration,
    startTime,
    commitTime,
  });
};

const parseIntOr = (value, fallback) => {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
};

const ProfiledSection = ({ id, children }) => {
  if (process.env.NODE_ENV !== "development") return children;
  return (
    <Profiler id={id} onRender={onRender}>
      {children}
    </Profiler>
  );
};

function App() {
  const { runs, selectedRun, setSelectedRun } = useRuns();
  const [historyRangeStart, setHistoryRangeStart] = useState(-50);
  const [historyRangeEnd, setHistoryRangeEnd] = useState(-1);

  return (
    <RunHistoryProvider runId={selectedRun} {...DASHBOARD_HISTORY_CONFIG}>
      <AppContent
        runs={runs}
        selectedRun={selectedRun}
        setSelectedRun={setSelectedRun}
        historyRangeStart={historyRangeStart}
        historyRangeEnd={historyRangeEnd}
        setHistoryRangeStart={setHistoryRangeStart}
        setHistoryRangeEnd={setHistoryRangeEnd}
      />
    </RunHistoryProvider>
  );
}

const useCurrentRun = (runs, selectedRun) => {
  const { run } = useRunState();
  return run || runs.find((r) => r.run_id === selectedRun);
};

const ConnectionAndSelector = ({
  runs,
  selectedRun,
  setSelectedRun,
  historyRangeStart,
  historyRangeEnd,
  setHistoryRangeStart,
  setHistoryRangeEnd,
}) => {
  const { isConnected } = useRunConnection();
  const { lastUpdate } = useRunHeartbeat();

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />
      <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5} sx={{ mb: 2 }}>
        <TextField
          size="small"
          type="number"
          label="History Start"
          value={historyRangeStart}
          onChange={(event) => setHistoryRangeStart((prev) => parseIntOr(event.target.value, prev))}
          inputProps={{ step: 1 }}
          helperText="Inclusive index (negative = relative to newest)"
        />
        <TextField
          size="small"
          type="number"
          label="History End"
          value={historyRangeEnd}
          onChange={(event) => setHistoryRangeEnd((prev) => parseIntOr(event.target.value, prev))}
          inputProps={{ step: 1 }}
          helperText="Inclusive index (default -1 = newest)"
        />
      </Stack>
    </>
  );
};

const resolveHistoryIndex = (index, len) => {
  if (index < 0) return len + index;
  return index;
};

const sliceHistoryByRange = (samples, startInclusive, endInclusive) => {
  if (!Array.isArray(samples) || samples.length === 0) return [];
  const len = samples.length;
  const rawStart = resolveHistoryIndex(startInclusive, len);
  const rawEnd = resolveHistoryIndex(endInclusive, len);
  const clampedStart = Math.max(0, Math.min(len - 1, rawStart));
  const clampedEnd = Math.max(0, Math.min(len - 1, rawEnd));
  const start = Math.min(clampedStart, clampedEnd);
  const end = Math.max(clampedStart, clampedEnd);
  return samples.slice(start, end + 1);
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

const ObservableSection = ({ runs, selectedRun, historyRangeStart, historyRangeEnd }) => {
  const currentRun = runs.find((r) => r.run_id === selectedRun) || null;
  const { history, latestAggregated } = useRunAggregated();
  const { isConnected } = useRunConnection();
  const observableImplementation =
    currentRun?.observable_implementation ||
    currentRun?.integration_params?.observable_implementation ||
    latestAggregated?.observable_implementation ||
    "scalar";

  const fullSamples = useMemo(
    () =>
      history
        .slice()
        .reverse()
        .map((item) => deriveObservableMetric(item.aggregated_observable || {}, observableImplementation)),
    [history, observableImplementation],
  );
  const derivedSamples = useMemo(
    () => sliceHistoryByRange(fullSamples, historyRangeStart, historyRangeEnd),
    [fullSamples, historyRangeStart, historyRangeEnd],
  );

  return (
    <ObservablePanel
      run={currentRun}
      samples={derivedSamples}
      totalSnapshots={fullSamples.length}
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

const AppContent = ({
  runs,
  selectedRun,
  setSelectedRun,
  historyRangeStart,
  historyRangeEnd,
  setHistoryRangeStart,
  setHistoryRangeEnd,
}) => (
  <Container maxWidth="xl" sx={{ py: 3 }}>
    <Box sx={{ mb: 3 }}>
      <Typography variant="h3" component="h1" gutterBottom>
        Gammaboard
      </Typography>
      <Typography variant="body2" color="text.secondary">
        Real-time Monte Carlo simulation monitoring
      </Typography>
    </Box>

    <ProfiledSection id="ConnectionAndSelector">
      <ConnectionAndSelector
        runs={runs}
        selectedRun={selectedRun}
        setSelectedRun={setSelectedRun}
        historyRangeStart={historyRangeStart}
        historyRangeEnd={historyRangeEnd}
        setHistoryRangeStart={setHistoryRangeStart}
        setHistoryRangeEnd={setHistoryRangeEnd}
      />
    </ProfiledSection>
    <ProfiledSection id="RunPanels">
      <RunPanels runs={runs} selectedRun={selectedRun} />
    </ProfiledSection>
    <ProfiledSection id="ObservableSection">
      <ObservableSection
        runs={runs}
        selectedRun={selectedRun}
        historyRangeStart={historyRangeStart}
        historyRangeEnd={historyRangeEnd}
      />
    </ProfiledSection>
    <ProfiledSection id="SamplerSection">
      <SamplerSection runs={runs} selectedRun={selectedRun} />
    </ProfiledSection>
    <ProfiledSection id="WorkerLogsSection">
      <WorkerLogsSection selectedRun={selectedRun} />
    </ProfiledSection>
    <ProfiledSection id="WorkersPanel">
      <WorkersPanel runId={selectedRun} />
    </ProfiledSection>
  </Container>
);

export default App;
