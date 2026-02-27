import { useEffect, useMemo, useState } from "react";
import { Box, Card, CardContent, Chip, Grid, Paper, Typography } from "@mui/material";
import { DataGrid } from "@mui/x-data-grid";
import { fetchWorkers } from "../services/api";
import { formatDateTime } from "../utils/formatters";
import JsonFallback from "./JsonFallback";
import UnsupportedImplementationPanel from "./common/UnsupportedImplementationPanel";

const ageSeconds = (value) => {
  if (!value) return null;
  const ts = new Date(value).getTime();
  if (!Number.isFinite(ts)) return null;
  return Math.max(0, Math.floor((Date.now() - ts) / 1000));
};

const statusColor = (status) => {
  switch ((status || "").toLowerCase()) {
    case "active":
      return "success";
    case "draining":
      return "warning";
    case "inactive":
      return "default";
    default:
      return "default";
  }
};

const fmtMs = (value) => {
  if (!Number.isFinite(Number(value))) return "-";
  return Number(value).toFixed(4);
};

const fmtInt = (value) => {
  if (value == null || !Number.isFinite(Number(value))) return "-";
  return Number(value).toLocaleString();
};

const toObjectOrNull = (value) => (value && typeof value === "object" && !Array.isArray(value) ? value : null);
const rollingMean = (metric) => {
  if (Number.isFinite(Number(metric))) return Number(metric);
  const obj = toObjectOrNull(metric);
  if (!obj) return null;
  const mean = Number(obj.mean);
  return Number.isFinite(mean) ? mean : null;
};
const evaluatorMetrics = (worker) => toObjectOrNull(worker?.evaluator_metrics) || {};
const samplerMetrics = (worker) => toObjectOrNull(worker?.sampler_metrics) || {};
const evaluatorIdleRatio = (worker) => {
  const idleProfile = toObjectOrNull(evaluatorMetrics(worker).idle_profile) || {};
  const ratio = Number(idleProfile.idle_ratio);
  if (!Number.isFinite(ratio)) return null;
  return Math.min(1, Math.max(0, ratio));
};

const fmtDiagnosticValue = (value) => {
  if (Number.isFinite(Number(value))) return Number(value).toLocaleString();
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
};

const SamplerRuntimePanel = ({ runtimeMetrics }) => {
  const root = toObjectOrNull(runtimeMetrics) || {};
  const rolling = toObjectOrNull(root.rolling);

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Sampler Runtime Metrics
        </Typography>
        <Grid container spacing={2}>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              batch_size_current
            </Typography>
            <Typography variant="h5">{root.batch_size_current ?? "n/a"}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              eval_ms_per_sample
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.eval_ms_per_sample) != null
                ? rollingMean(rolling?.eval_ms_per_sample).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              eval_ms_per_batch
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.eval_ms_per_batch) != null
                ? rollingMean(rolling?.eval_ms_per_batch).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              sampler_produce_ms_per_sample
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.sampler_produce_ms_per_sample) != null
                ? rollingMean(rolling?.sampler_produce_ms_per_sample).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              sampler_ingest_ms_per_sample
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.sampler_ingest_ms_per_sample) != null
                ? rollingMean(rolling?.sampler_ingest_ms_per_sample).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              queue_remaining_ratio
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.queue_remaining_ratio) != null
                ? rollingMean(rolling?.queue_remaining_ratio).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              batches_consumed_per_tick
            </Typography>
            <Typography variant="h5">
              {rollingMean(rolling?.batches_consumed_per_tick) != null
                ? rollingMean(rolling?.batches_consumed_per_tick).toFixed(4)
                : "n/a"}
            </Typography>
          </Grid>
        </Grid>
      </CardContent>
    </Card>
  );
};

const HavanaDiagnosticsPanel = ({ engineDiagnostics }) => {
  const diagnostics = toObjectOrNull(engineDiagnostics) || {};
  const chiSq = Number.isFinite(Number(diagnostics?.chi_sq)) ? Number(diagnostics.chi_sq) : null;
  const otherFields = Object.entries(diagnostics).filter(([key]) => key !== "chi_sq");

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Havana Engine Diagnostics
        </Typography>
        <Grid container spacing={2}>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              chi_sq
            </Typography>
            <Typography variant="h5">{chiSq == null ? "n/a" : chiSq.toFixed(6)}</Typography>
          </Grid>
          {otherFields.map(([key, value]) => (
            <Grid key={key} item xs={12} sm={6} md={4}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                {key}
              </Typography>
              <Typography variant="h6">{fmtDiagnosticValue(value)}</Typography>
            </Grid>
          ))}
        </Grid>
      </CardContent>
    </Card>
  );
};

const NaiveMonteCarloDiagnosticsPanel = ({ engineDiagnostics }) => {
  const merged = toObjectOrNull(engineDiagnostics) || {};

  if (Object.keys(merged).length === 0) {
    return (
      <Card variant="outlined">
        <CardContent>
          <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
            naive_monte_carlo Diagnostics
          </Typography>
          <Typography variant="body2" color="text.secondary">
            No custom diagnostics reported.
          </Typography>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          naive_monte_carlo Diagnostics
        </Typography>
        <Grid container spacing={2}>
          {Object.entries(merged).map(([key, val]) => (
            <Grid key={key} item xs={12} sm={6} md={4}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                {key}
              </Typography>
              <Typography variant="h6">{fmtDiagnosticValue(val)}</Typography>
            </Grid>
          ))}
        </Grid>
      </CardContent>
    </Card>
  );
};

const SamplerDiagnosticsCustomPanel = ({ worker }) => {
  if (!worker) {
    return (
      <Card variant="outlined">
        <CardContent>
          <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
            Sampler Diagnostics
          </Typography>
          <Typography variant="body2" color="text.secondary">
            No sampler_aggregator worker is currently registered for this run.
          </Typography>
        </CardContent>
      </Card>
    );
  }

  if (worker.implementation === "havana") {
    return <HavanaDiagnosticsPanel engineDiagnostics={worker.sampler_engine_diagnostics} />;
  }

  if (worker.implementation === "naive_monte_carlo") {
    return <NaiveMonteCarloDiagnosticsPanel engineDiagnostics={worker.sampler_engine_diagnostics} />;
  }

  return <UnsupportedImplementationPanel kind="sampler diagnostics" implementation={worker.implementation} />;
};

const WorkersPanel = ({ runId, refreshMs = 3000 }) => {
  const [workers, setWorkers] = useState([]);

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const load = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const data = await fetchWorkers(runId);
        if (!cancelled) setWorkers(Array.isArray(data) ? data : []);
      } catch {
        if (!cancelled) setWorkers([]);
      } finally {
        inFlight = false;
      }
    };

    load();
    const timer = setInterval(load, refreshMs);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [refreshMs, runId]);

  const evaluatorWorkers = useMemo(() => workers.filter((worker) => worker.role === "evaluator"), [workers]);
  const samplerWorkers = useMemo(() => workers.filter((worker) => worker.role === "sampler_aggregator"), [workers]);

  const evaluatorRows = useMemo(
    () =>
      evaluatorWorkers.map((worker) => ({
        id: worker.worker_id,
        ...worker,
        batches_completed: evaluatorMetrics(worker).batches_completed,
        samples_evaluated: evaluatorMetrics(worker).samples_evaluated,
        avg_time_per_sample_ms: evaluatorMetrics(worker).avg_time_per_sample_ms,
        std_time_per_sample_ms: evaluatorMetrics(worker).std_time_per_sample_ms,
        idle_ratio: evaluatorIdleRatio(worker),
        last_seen_age_s: ageSeconds(worker.last_seen),
      })),
    [evaluatorWorkers],
  );
  const selectedSamplerWorker = samplerWorkers[0] || null;

  const evaluatorActiveCount = evaluatorWorkers.filter((w) => (w.status || "").toLowerCase() === "active").length;
  const samplerActiveCount = samplerWorkers.filter((w) => (w.status || "").toLowerCase() === "active").length;

  const evaluatorColumns = useMemo(
    () => [
      { field: "worker_id", headerName: "Worker", minWidth: 220, flex: 1 },
      { field: "node_id", headerName: "Node", minWidth: 140, flex: 0.7 },
      { field: "implementation", headerName: "Implementation", minWidth: 190, flex: 1 },
      {
        field: "status",
        headerName: "Status",
        minWidth: 130,
        flex: 0.6,
        renderCell: (params) => (
          <Chip size="small" label={params.value || "unknown"} color={statusColor(params.value)} variant="outlined" />
        ),
      },
      {
        field: "batches_completed",
        headerName: "Batches",
        minWidth: 100,
        flex: 0.5,
        renderCell: (params) => (params.value == null ? "-" : String(params.value)),
      },
      {
        field: "samples_evaluated",
        headerName: "Samples",
        minWidth: 120,
        flex: 0.6,
        renderCell: (params) => (params.value == null ? "-" : Number(params.value).toLocaleString()),
      },
      {
        field: "avg_time_per_sample_ms",
        headerName: "Avg ms/sample",
        minWidth: 130,
        flex: 0.7,
        renderCell: (params) => fmtMs(params.value),
      },
      {
        field: "std_time_per_sample_ms",
        headerName: "Std ms/sample",
        minWidth: 130,
        flex: 0.7,
        renderCell: (params) => fmtMs(params.value),
      },
      {
        field: "idle_ratio",
        headerName: "Idle Ratio",
        minWidth: 110,
        flex: 0.6,
        renderCell: (params) => {
          const v = Number(params.value);
          return Number.isFinite(v) ? `${(v * 100).toFixed(1)}%` : "-";
        },
      },
      {
        field: "last_seen_age_s",
        headerName: "Last Seen (s)",
        minWidth: 130,
        flex: 0.6,
        renderCell: (params) => (params.value == null ? "-" : String(params.value)),
      },
      {
        field: "last_seen",
        headerName: "Last Seen",
        minWidth: 220,
        flex: 1,
        renderCell: (params) => formatDateTime(params.value, "-"),
      },
    ],
    [],
  );

  return (
    <Box sx={{ mb: 3 }}>
      <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 1 }}>
        <Typography variant="h6">Workers</Typography>
        <Typography variant="body2" color="text.secondary">
          {runId != null ? `Desired Run: #${runId} | ` : ""}
          Registered: {workers.length}
        </Typography>
      </Box>

      <Box sx={{ mb: 3 }}>
        <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 1 }}>
          <Typography variant="subtitle1">Sampler Aggregator Workers</Typography>
          <Typography variant="body2" color="text.secondary">
            Registered: {samplerWorkers.length} | Active: {samplerActiveCount}
          </Typography>
        </Box>
        <Card variant="outlined" sx={{ mb: 2 }}>
          <CardContent>
            <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
              Sampler Aggregator Worker
            </Typography>
            {selectedSamplerWorker ? (
              <Grid container spacing={2}>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    worker
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {selectedSamplerWorker.worker_id}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    node
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {selectedSamplerWorker.node_id || "-"}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    implementation
                  </Typography>
                  <Typography variant="body2">{selectedSamplerWorker.implementation || "unknown"}</Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    status
                  </Typography>
                  <Box sx={{ mt: 0.5 }}>
                    <Chip
                      size="small"
                      label={selectedSamplerWorker.status || "unknown"}
                      color={statusColor(selectedSamplerWorker.status)}
                      variant="outlined"
                    />
                  </Box>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    produced batches
                  </Typography>
                  <Typography variant="body2">
                    {fmtInt(samplerMetrics(selectedSamplerWorker).produced_batches)}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    produced samples
                  </Typography>
                  <Typography variant="body2">
                    {fmtInt(samplerMetrics(selectedSamplerWorker).produced_samples)}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    produce avg ms/sample
                  </Typography>
                  <Typography variant="body2">
                    {fmtMs(samplerMetrics(selectedSamplerWorker).avg_produce_time_per_sample_ms)}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    ingest avg ms/sample
                  </Typography>
                  <Typography variant="body2">
                    {fmtMs(samplerMetrics(selectedSamplerWorker).avg_ingest_time_per_sample_ms)}
                  </Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    last seen
                  </Typography>
                  <Typography variant="body2">{formatDateTime(selectedSamplerWorker.last_seen, "-")}</Typography>
                </Grid>
              </Grid>
            ) : (
              <Typography variant="body2" color="text.secondary">
                No sampler_aggregator worker is currently registered for this run.
              </Typography>
            )}
          </CardContent>
        </Card>

        <Box sx={{ mb: 2 }}>
          <SamplerRuntimePanel runtimeMetrics={selectedSamplerWorker?.sampler_runtime_metrics} />
        </Box>
        <Box sx={{ mb: 2 }}>
          <SamplerDiagnosticsCustomPanel worker={selectedSamplerWorker} />
        </Box>
        <JsonFallback
          title="sampler diagnostics JSON"
          data={{
            worker_id: selectedSamplerWorker?.worker_id ?? null,
            implementation: selectedSamplerWorker?.implementation ?? null,
            runtime_metrics: selectedSamplerWorker?.sampler_runtime_metrics ?? null,
            engine_diagnostics: selectedSamplerWorker?.sampler_engine_diagnostics ?? null,
          }}
        />
      </Box>

      <Box>
        <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 1 }}>
          <Typography variant="subtitle1">Evaluator Workers</Typography>
          <Typography variant="body2" color="text.secondary">
            Registered: {evaluatorWorkers.length} | Active: {evaluatorActiveCount}
          </Typography>
        </Box>
        <Paper variant="outlined" sx={{ height: { xs: 280, md: 330 }, mb: 2 }}>
          <DataGrid
            rows={evaluatorRows}
            columns={evaluatorColumns}
            density="compact"
            hideFooter
            disableColumnMenu
            disableMultipleRowSelection
            disableRowSelectionOnClick
          />
        </Paper>
      </Box>
    </Box>
  );
};

export default WorkersPanel;
