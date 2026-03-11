import { useEffect, useMemo, useState } from "react";
import { Alert, Box, FormControl, InputLabel, MenuItem, Paper, Select, Stack, Typography } from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import RunSelector from "./RunSelector";
import SampleChart from "./SampleChart";
import JsonFallback from "./JsonFallback";
import { useRunPerformanceSummary } from "../hooks/useRunPerformanceSummary";

const buildPerformanceSamples = (entries, role) => {
  if (!Array.isArray(entries) || entries.length === 0) return [];
  return entries
    .map((entry) => {
      const createdAt = entry.created_at || null;
      const createdAtMs = createdAt ? Date.parse(createdAt) : NaN;
      const sampleCount = Number.isFinite(createdAtMs) ? createdAtMs : Number(entry.id);
      if (!Number.isFinite(sampleCount)) return null;
      const metrics = entry.metrics || {};
      const mean =
        role === "sampler_aggregator"
          ? Number(metrics.avg_produce_time_per_sample_ms)
          : Number(metrics.avg_time_per_sample_ms);
      const stderr =
        role === "sampler_aggregator"
          ? Number(metrics.std_produce_time_per_sample_ms)
          : Number(metrics.std_time_per_sample_ms);
      if (!Number.isFinite(mean)) return null;
      const safeStd = Number.isFinite(stderr) ? Math.abs(stderr) : 0;
      return {
        sampleCount,
        mean,
        stderr: safeStd,
        lower: mean - safeStd,
        upper: mean + safeStd,
        spread: safeStd * 2,
      };
    })
    .filter(Boolean)
    .reverse();
};

const latestEntryForWorker = (entries, workerId) =>
  (Array.isArray(entries) ? entries : []).find((entry) => entry.worker_id === workerId) || null;

const PerformanceSection = ({ title, entries, role, isConnected }) => {
  const samples = useMemo(() => buildPerformanceSamples(entries, role), [entries, role]);
  if (samples.length === 0) return null;

  const formatTimestamp = (value) => {
    const dt = new Date(Number(value));
    if (Number.isNaN(dt.getTime())) return String(value);
    return dt.toLocaleString();
  };

  return (
    <SampleChart
      samples={samples}
      isConnected={isConnected}
      hasRun
      target={null}
      title={title}
      lineColor={role === "sampler_aggregator" ? "#6a1b9a" : "#1565c0"}
      bandColor={role === "sampler_aggregator" ? "#6a1b9a" : "#1565c0"}
      targetLabel=""
      xAxisLabel="Snapshot time"
      yAxisLabel="ms/sample"
      sampleLabel="Snapshot time"
      valueLabel="ms/sample"
      showStdErr
      showErrorBand
      showTargetLine={false}
      showTargetSummary={false}
      xTickFormatter={formatTimestamp}
      sampleFormatter={formatTimestamp}
    />
  );
};

const workerLabel = (workerId, roles) => {
  const roleList = Array.from(roles).sort().join(", ");
  return roleList ? `${workerId} (${roleList})` : workerId;
};

const PerformanceWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected }) => {
  const { evaluatorEntries, samplerEntries } = useRunPerformanceSummary({
    runId: selectedRun,
    limit: 200,
    pollMs: 5000,
  });
  const [selectedWorkerId, setSelectedWorkerId] = useState("");

  const workers = useMemo(() => {
    const map = new Map();
    for (const entry of [...evaluatorEntries, ...samplerEntries]) {
      if (!entry?.worker_id) continue;
      if (!map.has(entry.worker_id)) map.set(entry.worker_id, new Set());
      map
        .get(entry.worker_id)
        .add(entry.metrics?.avg_produce_time_per_sample_ms != null ? "sampler_aggregator" : "evaluator");
    }
    return Array.from(map.entries()).map(([workerId, roles]) => ({ workerId, roles }));
  }, [evaluatorEntries, samplerEntries]);

  useEffect(() => {
    if (workers.length === 0) {
      setSelectedWorkerId("");
      return;
    }
    if (!workers.some((worker) => worker.workerId === selectedWorkerId)) {
      setSelectedWorkerId(workers[0].workerId);
    }
  }, [workers, selectedWorkerId]);

  const selectedEvaluatorEntries = useMemo(
    () => evaluatorEntries.filter((entry) => entry.worker_id === selectedWorkerId),
    [evaluatorEntries, selectedWorkerId],
  );
  const selectedSamplerEntries = useMemo(
    () => samplerEntries.filter((entry) => entry.worker_id === selectedWorkerId),
    [samplerEntries, selectedWorkerId],
  );

  const selectedWorker = workers.find((worker) => worker.workerId === selectedWorkerId) || null;
  const latestPayload = {
    evaluator: latestEntryForWorker(selectedEvaluatorEntries, selectedWorkerId),
    sampler_aggregator: latestEntryForWorker(selectedSamplerEntries, selectedWorkerId),
  };

  if (!Array.isArray(runs) || runs.length === 0) {
    return (
      <>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <EmptyStateCard title="No runs available" message="Create a run to inspect persisted performance history." />
      </>
    );
  }

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />

      {!selectedRun ? (
        <EmptyStateCard title="Select a run" message="Pick a run to inspect persisted worker performance history." />
      ) : (
        <>
          <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
            <Typography variant="h6" gutterBottom>
              Performance
            </Typography>
            {workers.length === 0 ? (
              <EmptyStateCard
                title="No performance history yet"
                message="Wait for evaluator or sampler performance snapshots to be recorded for this run."
              />
            ) : (
              <Stack spacing={2}>
                <FormControl size="small" sx={{ maxWidth: 420 }}>
                  <InputLabel id="performance-worker-select-label">Worker</InputLabel>
                  <Select
                    labelId="performance-worker-select-label"
                    value={selectedWorkerId}
                    label="Worker"
                    onChange={(event) => setSelectedWorkerId(event.target.value)}
                  >
                    {workers.map((worker) => (
                      <MenuItem key={worker.workerId} value={worker.workerId}>
                        {workerLabel(worker.workerId, worker.roles)}
                      </MenuItem>
                    ))}
                  </Select>
                </FormControl>
                {selectedWorker ? (
                  <Typography variant="body2" color="text.secondary">
                    Selected worker: <strong>{selectedWorker.workerId}</strong>
                  </Typography>
                ) : null}
              </Stack>
            )}
          </Paper>

          {selectedWorkerId && selectedEvaluatorEntries.length === 0 && selectedSamplerEntries.length === 0 ? (
            <Alert severity="info">No persisted performance entries found for the selected worker on this run.</Alert>
          ) : null}

          <PerformanceSection
            title="Evaluator ms/sample history"
            entries={selectedEvaluatorEntries}
            role="evaluator"
            isConnected={isConnected}
          />
          <PerformanceSection
            title="Sampler ms/sample history"
            entries={selectedSamplerEntries}
            role="sampler_aggregator"
            isConnected={isConnected}
          />

          <Box sx={{ mt: 2 }}>
            <JsonFallback title="latest performance snapshot JSON" data={latestPayload} />
          </Box>
        </>
      )}
    </>
  );
};

export default PerformanceWorkspace;
