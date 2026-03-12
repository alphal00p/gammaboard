import { Alert, Box, Container, Tab, Tabs, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg";
import ConnectionStatus from "./components/ConnectionStatus";
import EvaluatorPanel from "./components/EvaluatorPanel";
import HistoryRangeControls from "./components/HistoryRangeControls";
import ObservablePanel from "./components/ObservablePanel";
import RunInfo from "./components/RunInfo";
import RunSelector from "./components/RunSelector";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import LogsWorkspace from "./components/LogsWorkspace";
import PerformanceWorkspace from "./components/PerformanceWorkspace";
import WorkersWorkspace from "./components/WorkersWorkspace";
import RunScopedWorkspace from "./components/common/RunScopedWorkspace";
import { RunHistoryProvider, useRunHistory } from "./context/RunHistoryContext";
import { useRuns } from "./hooks/useRuns";
import { useRunPerformanceSummary } from "./hooks/useRunPerformanceSummary";
import { useWorkersData } from "./hooks/useWorkersData";
import { deriveObservableImplementation } from "./utils/config";
import { deriveObservableMetric } from "./viewmodels/observable";

const DASHBOARD_HISTORY_CONFIG = {
  historyBufferMax: 100,
  workQueueStatsLimit: 200,
  pollIntervalMs: 5000,
};

const DashboardHeader = () => (
  <Box sx={{ mb: 3 }}>
    <Box
      component="img"
      src={gammaboardLogo}
      alt="Gammaboard"
      sx={{ display: "block", width: "min(100%, 320px)", height: "auto", mb: 1 }}
    />
    <Typography variant="body2" color="text.secondary">
      Real-time Monte Carlo simulation monitoring
    </Typography>
  </Box>
);

const rollingMean = (metric) => {
  if (Number.isFinite(Number(metric))) return Number(metric);
  if (!metric || typeof metric !== "object" || Array.isArray(metric)) return null;
  const mean = Number(metric.mean);
  return Number.isFinite(mean) ? mean : null;
};

const deriveSamplerRuntimeSummary = (workers, runId, latestSamplerEntry) => {
  if (!runId) return null;
  const list = Array.isArray(workers) ? workers : [];
  const samplerWorker =
    list.find(
      (worker) =>
        worker.current_run_id === runId &&
        worker.current_role === "sampler_aggregator" &&
        String(worker.status || "").toLowerCase() === "active",
    ) ||
    list.find((worker) => worker.current_run_id === runId && worker.current_role === "sampler_aggregator") ||
    list.find((worker) => worker.desired_run_id === runId && worker.desired_role === "sampler_aggregator") ||
    null;
  const runtimeMetrics = samplerWorker?.sampler_runtime_metrics ?? latestSamplerEntry?.runtime_metrics ?? null;
  const samplerMetrics = latestSamplerEntry?.metrics ?? samplerWorker?.sampler_metrics ?? null;
  const rolling = runtimeMetrics?.rolling || {};
  const remainingRatio = rollingMean(rolling.queue_remaining_ratio);
  const targetQueueRemaining = Number(runtimeMetrics?.target_queue_remaining_ratio);
  return {
    current_batch_size: runtimeMetrics?.batch_size_current ?? null,
    target_queue_remaining_ratio: Number.isFinite(targetQueueRemaining) ? targetQueueRemaining : null,
    actual_queue_remaining_ratio: remainingRatio,
    queue_remaining_delta:
      Number.isFinite(remainingRatio) && Number.isFinite(targetQueueRemaining)
        ? remainingRatio - targetQueueRemaining
        : null,
    actual_eval_ms_per_sample: rollingMean(rolling.eval_ms_per_sample),
    actual_eval_ms_per_batch: rollingMean(rolling.eval_ms_per_batch),
    produce_ms_per_sample: Number.isFinite(Number(samplerMetrics?.avg_produce_time_per_sample_ms))
      ? Number(samplerMetrics.avg_produce_time_per_sample_ms)
      : rollingMean(rolling.sampler_produce_ms_per_sample),
    ingest_ms_per_sample: Number.isFinite(Number(samplerMetrics?.avg_ingest_time_per_sample_ms))
      ? Number(samplerMetrics.avg_ingest_time_per_sample_ms)
      : rollingMean(rolling.sampler_ingest_ms_per_sample),
    produced_batches: samplerMetrics?.produced_batches ?? runtimeMetrics?.produced_batches_total ?? null,
    produced_samples: samplerMetrics?.produced_samples ?? runtimeMetrics?.produced_samples_total ?? null,
    ingested_batches: samplerMetrics?.ingested_batches ?? runtimeMetrics?.ingested_batches_total ?? null,
    ingested_samples: samplerMetrics?.ingested_samples ?? runtimeMetrics?.ingested_samples_total ?? null,
  };
};

const RunModeContent = ({ runs, workers, selectedRun, setSelectedRun, historyRange, setHistoryRange }) => {
  const { isConnected, lastUpdate, history, latestAggregated, workQueueStats } = useRunHistory();
  const currentRun = runs.find((entry) => entry.run_id === selectedRun);
  const { latestSampler } = useRunPerformanceSummary({ runId: selectedRun, pollMs: 5000 });
  const samplerRuntimeSummary = useMemo(
    () => deriveSamplerRuntimeSummary(workers, selectedRun, latestSampler),
    [workers, selectedRun, latestSampler],
  );
  const latestObservablePayload = latestAggregated?.aggregated_observable ?? currentRun?.current_observable ?? null;
  const observableImplementation = deriveObservableImplementation(
    currentRun?.integration_params?.evaluator,
    latestObservablePayload,
    "scalar",
  );

  const fullSamples = useMemo(() => {
    const derived = history
      .slice()
      .reverse()
      .map((item) => deriveObservableMetric(item.aggregated_observable || {}, observableImplementation));
    if (derived.length > 0) return derived;
    if (!currentRun?.current_observable) return derived;
    return [deriveObservableMetric(currentRun.current_observable, observableImplementation)];
  }, [history, observableImplementation, currentRun]);

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

const RunsWorkspace = ({ runs, workers, selectedRun, setSelectedRun, isConnected, historyRange, setHistoryRange }) => {
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
          workers={workers}
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
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
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
        <Tab value="performance" label="Performance" />
        <Tab value="logs" label="Logs" />
      </Tabs>

      {mode === "runs" ? (
        <RunsWorkspace
          runs={runs}
          workers={workersData.workers}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
          historyRange={historyRange}
          setHistoryRange={setHistoryRange}
        />
      ) : mode === "workers" ? (
        <WorkersWorkspace
          workers={workersData.workers}
          isConnected={workersData.isConnected}
          lastUpdate={workersData.lastUpdate}
          error={workersData.error}
        />
      ) : mode === "performance" ? (
        <PerformanceWorkspace
          runs={runs}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
        />
      ) : (
        <LogsWorkspace
          runs={runs}
          workers={workersData.workers}
          selectedRun={selectedLogRun}
          setSelectedRun={setSelectedLogRun}
          isConnected={isConnected}
        />
      )}
    </Container>
  );
}

export default App;
