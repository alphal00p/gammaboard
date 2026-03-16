import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  FormControl,
  Grid,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  Typography,
} from "@mui/material";
import ConnectionStatus from "./ConnectionStatus";
import EmptyStateCard from "./common/EmptyStateCard";
import RunSelector from "./RunSelector";
import SampleChart from "./SampleChart";
import JsonFallback from "./JsonFallback";
import { useRunPerformanceSummary } from "../hooks/useRunPerformanceSummary";
import { formatDateTime } from "../utils/formatters";

const buildPerformanceSamples = (entries, { meanKey, stderrKey }) => {
  if (!Array.isArray(entries) || entries.length === 0) return [];
  return entries
    .map((entry) => {
      const createdAt = entry.created_at || null;
      const createdAtMs = createdAt ? Date.parse(createdAt) : NaN;
      const sampleCount = Number.isFinite(createdAtMs) ? createdAtMs : Number(entry.id);
      if (!Number.isFinite(sampleCount)) return null;
      const metrics = entry.metrics || {};
      const mean = Number(metrics[meanKey]);
      const stderr = Number(metrics[stderrKey]);
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

const buildOverallSamplerThroughputSamples = (entries) =>
  (Array.isArray(entries) ? entries : [])
    .map((entry) => {
      const createdAt = entry?.created_at ? Date.parse(entry.created_at) : NaN;
      const completedSamplesPerSecond = Number(entry?.runtime_metrics?.completed_samples_per_second);
      const snapshotId = Number(entry?.id);
      if (!Number.isFinite(createdAt) || !Number.isFinite(completedSamplesPerSecond)) return null;
      return {
        sampleCount: createdAt,
        mean: completedSamplesPerSecond,
        value: completedSamplesPerSecond,
        stderr: 0,
        lower: completedSamplesPerSecond,
        upper: completedSamplesPerSecond,
        spread: 0,
        snapshotId: Number.isFinite(snapshotId) ? snapshotId : null,
      };
    })
    .filter(Boolean)
    .sort((a, b) => a.sampleCount - b.sampleCount || (a.snapshotId ?? 0) - (b.snapshotId ?? 0));

const latestEntryForWorker = (entries, workerId) =>
  (Array.isArray(entries) ? entries : []).find((entry) => entry.worker_id === workerId) || null;

const metricValue = (entry, key) => {
  const value = Number(entry?.metrics?.[key]);
  return Number.isFinite(value) ? value : null;
};

const fmtMetric = (value, digits = 4) => {
  if (!Number.isFinite(Number(value))) return "n/a";
  return `${Number(value).toFixed(digits)} ms`;
};

const fmtCount = (value) => {
  const num = Number(value);
  return Number.isFinite(num) ? num.toLocaleString() : "n/a";
};

const LatestSnapshotPanel = ({ title, snapshotAt, fields }) => (
  <Card variant="outlined" sx={{ height: "100%" }}>
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 0.5 }}>
        {title}
      </Typography>
      <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 2 }}>
        latest snapshot: {formatDateTime(snapshotAt, "n/a")}
      </Typography>
      <Grid container spacing={2}>
        {fields.map((field) => (
          <Grid key={field.label} item xs={12} sm={6} md={field.md ?? 3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              {field.label}
            </Typography>
            <Typography variant="h6" sx={{ fontFamily: "monospace" }}>
              {field.value}
            </Typography>
          </Grid>
        ))}
      </Grid>
    </CardContent>
  </Card>
);

const PerformanceSection = ({ title, entries, meanKey, stderrKey, isConnected, color }) => {
  const samples = useMemo(
    () => buildPerformanceSamples(entries, { meanKey, stderrKey }),
    [entries, meanKey, stderrKey],
  );
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
      lineColor={color}
      bandColor={color}
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

const ThroughputSection = ({ title, entries, isConnected, color }) => {
  const samples = useMemo(() => buildOverallSamplerThroughputSamples(entries), [entries]);
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
      lineColor={color}
      bandColor={color}
      targetLabel=""
      xAxisLabel="Snapshot time"
      yAxisLabel="completed samples/s"
      sampleLabel="Snapshot time"
      valueLabel="completed samples/s"
      showStdErr={false}
      showErrorBand={false}
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
    if (selectedWorkerId && !workers.some((worker) => worker.workerId === selectedWorkerId)) {
      setSelectedWorkerId("");
    }
  }, [workers, selectedWorkerId]);

  const selectedEvaluatorEntries = useMemo(
    () => (selectedWorkerId ? evaluatorEntries.filter((entry) => entry.worker_id === selectedWorkerId) : []),
    [evaluatorEntries, selectedWorkerId],
  );
  const selectedSamplerEntries = useMemo(
    () => (selectedWorkerId ? samplerEntries.filter((entry) => entry.worker_id === selectedWorkerId) : []),
    [samplerEntries, selectedWorkerId],
  );

  const selectedWorker = workers.find((worker) => worker.workerId === selectedWorkerId) || null;
  const latestPayload = {
    evaluator: latestEntryForWorker(selectedEvaluatorEntries, selectedWorkerId),
    sampler_aggregator: latestEntryForWorker(selectedSamplerEntries, selectedWorkerId),
  };
  const latestEvaluator = latestPayload.evaluator;
  const latestSampler = latestPayload.sampler_aggregator;

  if (!Array.isArray(runs) || runs.length === 0) {
    return (
      <>
        <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
        <EmptyStateCard
          title="No runs available"
          message="Create a run to inspect persisted node performance history."
        />
      </>
    );
  }

  return (
    <>
      <ConnectionStatus isConnected={isConnected} lastUpdate={null} />
      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />

      {!selectedRun ? (
        <EmptyStateCard title="Select a run" message="Pick a run to inspect persisted node performance history." />
      ) : (
        <>
          <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
            <Typography variant="h6" gutterBottom>
              Overall Performance
            </Typography>
            {samplerEntries.length === 0 && evaluatorEntries.length === 0 ? (
              <EmptyStateCard
                title="No performance history yet"
                message="Wait for evaluator or sampler performance snapshots to be recorded for this run."
              />
            ) : (
              <Stack spacing={1.5}>
                <Typography variant="body2" color="text.secondary">
                  Run-level charts aggregate persisted performance history across all nodes.
                </Typography>
              </Stack>
            )}
          </Paper>

          <ThroughputSection
            title="Overall completed samples per second"
            entries={samplerEntries}
            isConnected={isConnected}
            color="#ef6c00"
          />

          <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
            <Typography variant="h6" gutterBottom>
              Implementation Details
            </Typography>
            {workers.length === 0 ? (
              <Typography variant="body2" color="text.secondary">
                No node-specific performance snapshots have been recorded yet.
              </Typography>
            ) : (
              <Stack spacing={2}>
                <FormControl size="small" sx={{ maxWidth: 420 }}>
                  <InputLabel id="performance-worker-select-label">Node</InputLabel>
                  <Select
                    labelId="performance-worker-select-label"
                    value={selectedWorkerId}
                    label="Node"
                    displayEmpty
                    onChange={(event) => setSelectedWorkerId(event.target.value)}
                  >
                    <MenuItem value="">
                      <em>No node selected</em>
                    </MenuItem>
                    {workers.map((worker) => (
                      <MenuItem key={worker.workerId} value={worker.workerId}>
                        {workerLabel(worker.workerId, worker.roles)}
                      </MenuItem>
                    ))}
                  </Select>
                </FormControl>
                {selectedWorker ? (
                  <Typography variant="body2" color="text.secondary">
                    Selected node: <strong>{selectedWorker.workerId}</strong>
                  </Typography>
                ) : (
                  <Typography variant="body2" color="text.secondary">
                    Select a node to inspect evaluator and sampler-specific runtime details.
                  </Typography>
                )}
              </Stack>
            )}
          </Paper>

          {selectedWorkerId && selectedEvaluatorEntries.length === 0 && selectedSamplerEntries.length === 0 ? (
            <Alert severity="info">No persisted performance entries found for the selected node on this run.</Alert>
          ) : null}

          {selectedWorkerId ? (
            <>
              <PerformanceSection
                title="Evaluator ms/sample history"
                entries={selectedEvaluatorEntries}
                meanKey="avg_time_per_sample_ms"
                stderrKey="std_time_per_sample_ms"
                isConnected={isConnected}
                color="#1565c0"
              />
              <PerformanceSection
                title="Sampler produce ms/sample history"
                entries={selectedSamplerEntries}
                meanKey="avg_produce_time_per_sample_ms"
                stderrKey="std_produce_time_per_sample_ms"
                isConnected={isConnected}
                color="#6a1b9a"
              />
            </>
          ) : null}

          {selectedWorkerId && (latestEvaluator || latestSampler) && (
            <Box sx={{ mt: 1, mb: 3 }}>
              <Typography variant="h6" gutterBottom>
                Latest Snapshot Summary
              </Typography>
              <Grid container spacing={2}>
                {latestEvaluator ? (
                  <Grid item xs={12}>
                    <LatestSnapshotPanel
                      title="Evaluator Snapshot"
                      snapshotAt={latestEvaluator.created_at}
                      fields={[
                        { label: "node", value: latestEvaluator.worker_id ?? "n/a", md: 3 },
                        {
                          label: "batches completed",
                          value: fmtCount(latestEvaluator.metrics?.batches_completed),
                          md: 3,
                        },
                        {
                          label: "samples evaluated",
                          value: fmtCount(latestEvaluator.metrics?.samples_evaluated),
                          md: 3,
                        },
                        {
                          label: "avg ms/sample",
                          value: fmtMetric(metricValue(latestEvaluator, "avg_time_per_sample_ms")),
                          md: 3,
                        },
                        {
                          label: "std ms/sample",
                          value: fmtMetric(metricValue(latestEvaluator, "std_time_per_sample_ms")),
                          md: 3,
                        },
                        {
                          label: "idle ratio",
                          value: Number.isFinite(Number(latestEvaluator.metrics?.idle_profile?.idle_ratio))
                            ? `${(Number(latestEvaluator.metrics.idle_profile.idle_ratio) * 100).toFixed(1)}%`
                            : "n/a",
                          md: 3,
                        },
                        { label: "run id", value: String(latestEvaluator.run_id ?? "n/a"), md: 3 },
                        { label: "snapshot id", value: String(latestEvaluator.id ?? "n/a"), md: 3 },
                      ]}
                    />
                  </Grid>
                ) : null}
                {latestSampler ? (
                  <Grid item xs={12}>
                    <LatestSnapshotPanel
                      title="Sampler Snapshot"
                      snapshotAt={latestSampler.created_at}
                      fields={[
                        { label: "node", value: latestSampler.worker_id ?? "n/a", md: 3 },
                        { label: "produced batches", value: fmtCount(latestSampler.metrics?.produced_batches), md: 3 },
                        { label: "produced samples", value: fmtCount(latestSampler.metrics?.produced_samples), md: 3 },
                        { label: "ingested batches", value: fmtCount(latestSampler.metrics?.ingested_batches), md: 3 },
                        { label: "ingested samples", value: fmtCount(latestSampler.metrics?.ingested_samples), md: 3 },
                        {
                          label: "produce ms/sample",
                          value: fmtMetric(metricValue(latestSampler, "avg_produce_time_per_sample_ms")),
                          md: 3,
                        },
                        {
                          label: "produce std",
                          value: fmtMetric(metricValue(latestSampler, "std_produce_time_per_sample_ms")),
                          md: 3,
                        },
                        {
                          label: "ingest ms/sample",
                          value: fmtMetric(metricValue(latestSampler, "avg_ingest_time_per_sample_ms")),
                          md: 3,
                        },
                        {
                          label: "ingest std",
                          value: fmtMetric(metricValue(latestSampler, "std_ingest_time_per_sample_ms")),
                          md: 3,
                        },
                        { label: "run id", value: String(latestSampler.run_id ?? "n/a"), md: 3 },
                        { label: "snapshot id", value: String(latestSampler.id ?? "n/a"), md: 3 },
                      ]}
                    />
                  </Grid>
                ) : null}
              </Grid>
            </Box>
          )}

          {selectedWorkerId ? (
            <Box sx={{ mt: 2 }}>
              <JsonFallback title="latest performance snapshot JSON" data={latestPayload} />
            </Box>
          ) : null}
        </>
      )}
    </>
  );
};

export default PerformanceWorkspace;
