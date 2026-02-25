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

const HavanaDiagnosticsPanel = ({ diagnostics }) => {
  const obj = toObjectOrNull(diagnostics);
  const chiSqFromObject = obj && Number.isFinite(Number(obj.chi_sq)) ? Number(obj.chi_sq) : null;
  const chiSq = Number.isFinite(Number(diagnostics)) ? Number(diagnostics) : chiSqFromObject;

  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Havana Diagnostics
        </Typography>
        <Grid container spacing={2}>
          <Grid item xs={12} sm={6} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              chi_sq
            </Typography>
            <Typography variant="h5">{chiSq == null ? "n/a" : chiSq.toFixed(6)}</Typography>
          </Grid>
        </Grid>
      </CardContent>
    </Card>
  );
};

const TestOnlyTrainingDiagnosticsPanel = ({ diagnostics }) => {
  const obj = toObjectOrNull(diagnostics);

  if (!obj || Object.keys(obj).length === 0) {
    return (
      <Card variant="outlined">
        <CardContent>
          <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
            test_only_training Diagnostics
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
          test_only_training Diagnostics
        </Typography>
        <Grid container spacing={2}>
          {Object.entries(obj).map(([key, val]) => (
            <Grid key={key} item xs={12} sm={6} md={4}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                {key}
              </Typography>
              <Typography variant="h6">
                {Number.isFinite(Number(val)) ? Number(val).toLocaleString() : String(val)}
              </Typography>
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
    return <HavanaDiagnosticsPanel diagnostics={worker.sampler_diagnostics} />;
  }

  if (worker.implementation === "test_only_training") {
    return <TestOnlyTrainingDiagnosticsPanel diagnostics={worker.sampler_diagnostics} />;
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
                  <Typography variant="body2">{fmtInt(selectedSamplerWorker.produced_batches)}</Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    produced samples
                  </Typography>
                  <Typography variant="body2">{fmtInt(selectedSamplerWorker.produced_samples)}</Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    produce avg ms/sample
                  </Typography>
                  <Typography variant="body2">{fmtMs(selectedSamplerWorker.avg_produce_time_per_sample_ms)}</Typography>
                </Grid>
                <Grid item xs={12} sm={6} md={3}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    ingest avg ms/sample
                  </Typography>
                  <Typography variant="body2">{fmtMs(selectedSamplerWorker.avg_ingest_time_per_sample_ms)}</Typography>
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
          <SamplerDiagnosticsCustomPanel worker={selectedSamplerWorker} />
        </Box>
        <JsonFallback
          title="sampler diagnostics JSON"
          data={{
            worker_id: selectedSamplerWorker?.worker_id ?? null,
            implementation: selectedSamplerWorker?.implementation ?? null,
            diagnostics: selectedSamplerWorker?.sampler_diagnostics ?? null,
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
