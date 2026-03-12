import { Alert, Box, Card, CardContent, Chip, Grid, Typography } from "@mui/material";
import { formatDateTime } from "../utils/formatters";
import JsonFallback from "./JsonFallback";
import UnsupportedImplementationPanel from "./common/UnsupportedImplementationPanel";

const toObjectOrNull = (value) => (value && typeof value === "object" && !Array.isArray(value) ? value : null);

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

const fmtMs = (value) => (Number.isFinite(Number(value)) ? Number(value).toFixed(4) : "n/a");
const fmtInt = (value) => (value == null || !Number.isFinite(Number(value)) ? "n/a" : Number(value).toLocaleString());

const evaluatorMetrics = (worker) => toObjectOrNull(worker?.evaluator_metrics) || {};
const samplerMetrics = (worker) => toObjectOrNull(worker?.sampler_metrics) || {};

const evaluatorIdleRatio = (worker) => {
  const idleProfile = toObjectOrNull(evaluatorMetrics(worker).idle_profile) || {};
  const ratio = Number(idleProfile.idle_ratio);
  if (!Number.isFinite(ratio)) return null;
  return Math.min(1, Math.max(0, ratio));
};

const rollingMean = (metric) => {
  if (Number.isFinite(Number(metric))) return Number(metric);
  const obj = toObjectOrNull(metric);
  if (!obj) return null;
  const mean = Number(obj.mean);
  return Number.isFinite(mean) ? mean : null;
};

const fmtDiagnosticValue = (value) => {
  if (Number.isFinite(Number(value))) return Number(value).toLocaleString();
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
};

const workerHasAssignedRun = (worker) => worker?.desired_run_id != null;
const workerIsInactive = (worker) => String(worker?.status || "").toLowerCase() === "inactive";
const currentRole = (worker) => worker?.current_role ?? null;
const currentRunId = (worker) => worker?.current_run_id ?? null;
const desiredRole = (worker) => worker?.desired_role ?? null;
const desiredRunId = (worker) => worker?.desired_run_id ?? null;

const unavailableMetricsMessage = (worker, roleLabel) => {
  if (currentRole(worker) == null || currentRunId(worker) == null) {
    if (!workerHasAssignedRun(worker)) {
      return `No run is currently assigned to this ${roleLabel} node. Metrics will appear after assignment.`;
    }
    return `This ${roleLabel} node is assigned but not currently active. Metrics will appear after the role starts.`;
  }
  if (workerIsInactive(worker)) {
    return `This ${roleLabel} node is inactive. Metrics will appear when the node becomes active again.`;
  }
  return null;
};

const MetricsUnavailableCard = ({ title, message }) => (
  <Card variant="outlined" sx={{ mb: 2 }}>
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary">
        {message}
      </Typography>
    </CardContent>
  </Card>
);

const WorkerOverviewPanel = ({ worker }) => (
  <Card variant="outlined" sx={{ mb: 2 }}>
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
        Node Overview
      </Typography>
      <Grid container spacing={2}>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            node_id
          </Typography>
          <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
            {worker.node_id || worker.worker_id}
          </Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            current_role
          </Typography>
          <Typography variant="body2">{currentRole(worker) || "None"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            status
          </Typography>
          <Box sx={{ mt: 0.5 }}>
            <Chip
              size="small"
              label={worker.status || "unknown"}
              color={statusColor(worker.status)}
              variant="outlined"
            />
          </Box>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            current_run_id
          </Typography>
          <Typography variant="body2">{currentRunId(worker) ?? "N/A"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            desired_role
          </Typography>
          <Typography variant="body2">{desiredRole(worker) || "None"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            desired_run_id
          </Typography>
          <Typography variant="body2">{desiredRunId(worker) ?? "N/A"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            implementation
          </Typography>
          <Typography variant="body2">{worker.implementation || "unknown"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            version
          </Typography>
          <Typography variant="body2">{worker.version || "n/a"}</Typography>
        </Grid>
        <Grid item xs={12} sm={6} md={3}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            last_seen
          </Typography>
          <Typography variant="body2">{formatDateTime(worker.last_seen, "-")}</Typography>
        </Grid>
      </Grid>
    </CardContent>
  </Card>
);

const EvaluatorMetricsPanel = ({ worker }) => {
  const unavailableMessage = unavailableMetricsMessage(worker, "evaluator");
  if (unavailableMessage) {
    return <MetricsUnavailableCard title="Evaluator Metrics" message={unavailableMessage} />;
  }

  const metrics = evaluatorMetrics(worker);
  const idle = evaluatorIdleRatio(worker);

  return (
    <Card variant="outlined" sx={{ mb: 2 }}>
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Evaluator Metrics
        </Typography>
        <Grid container spacing={2}>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              batches_completed
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.batches_completed)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              samples_evaluated
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.samples_evaluated)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              avg_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.avg_time_per_sample_ms)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              std_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.std_time_per_sample_ms)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              idle_ratio
            </Typography>
            <Typography variant="body2">
              {Number.isFinite(Number(idle)) ? `${(Number(idle) * 100).toFixed(1)}%` : "n/a"}
            </Typography>
          </Grid>
        </Grid>
      </CardContent>
    </Card>
  );
};

const SamplerMetricsPanel = ({ worker }) => {
  const unavailableMessage = unavailableMetricsMessage(worker, "sampler");
  if (unavailableMessage) {
    return <MetricsUnavailableCard title="Sampler Aggregator Metrics" message={unavailableMessage} />;
  }

  const metrics = samplerMetrics(worker);

  return (
    <Card variant="outlined" sx={{ mb: 2 }}>
      <CardContent>
        <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
          Sampler Aggregator Metrics
        </Typography>
        <Grid container spacing={2}>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              produced_batches
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.produced_batches)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              produced_samples
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.produced_samples)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              avg_produce_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.avg_produce_time_per_sample_ms)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              std_produce_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.std_produce_time_per_sample_ms)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              ingested_batches
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.ingested_batches)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              ingested_samples
            </Typography>
            <Typography variant="body2">{fmtInt(metrics.ingested_samples)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              avg_ingest_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.avg_ingest_time_per_sample_ms)}</Typography>
          </Grid>
          <Grid item xs={12} sm={6} md={3}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              std_ingest_time_per_sample_ms
            </Typography>
            <Typography variant="body2">{fmtMs(metrics.std_ingest_time_per_sample_ms)}</Typography>
          </Grid>
        </Grid>
      </CardContent>
    </Card>
  );
};

const SamplerRuntimePanel = ({ worker }) => {
  const unavailableMessage = unavailableMetricsMessage(worker, "sampler");
  if (unavailableMessage) {
    return <MetricsUnavailableCard title="Sampler Runtime Metrics" message={unavailableMessage} />;
  }

  const root = toObjectOrNull(worker?.sampler_runtime_metrics) || {};
  const rolling = toObjectOrNull(root.rolling);

  if (Object.keys(root).length === 0) {
    return (
      <MetricsUnavailableCard
        title="Sampler Runtime Metrics"
        message="No live sampler runtime metrics are currently available for this worker."
      />
    );
  }

  return (
    <Card variant="outlined" sx={{ mb: 2 }}>
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
    <Card variant="outlined" sx={{ mb: 2 }}>
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
      <Card variant="outlined" sx={{ mb: 2 }}>
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
    <Card variant="outlined" sx={{ mb: 2 }}>
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
      <Card variant="outlined" sx={{ mb: 2 }}>
        <CardContent>
          <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 0.5 }}>
            Sampler Diagnostics
          </Typography>
          <Typography variant="body2" color="text.secondary">
            No sampler_aggregator node is currently active.
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

const WorkerDetailsPanel = ({ worker }) => {
  if (!worker) return null;

  return (
    <>
      <WorkerOverviewPanel worker={worker} />

      {currentRole(worker) === "evaluator" ? (
        <>
          <EvaluatorMetricsPanel worker={worker} />
        </>
      ) : currentRole(worker) === "sampler_aggregator" ? (
        <>
          <SamplerMetricsPanel worker={worker} />
          <SamplerRuntimePanel worker={worker} />
          <SamplerDiagnosticsCustomPanel worker={worker} />
          <JsonFallback
            title="sampler diagnostics JSON"
            data={{
              node_id: worker.node_id ?? worker.worker_id ?? null,
              implementation: worker.implementation ?? null,
              runtime_metrics: worker.sampler_runtime_metrics ?? null,
              engine_diagnostics: worker.sampler_engine_diagnostics ?? null,
            }}
          />
        </>
      ) : (
        <Alert severity="info" sx={{ mb: 2 }}>
          No role-specific panels are available for this node while it is idle.
        </Alert>
      )}

      <Box sx={{ mt: 2 }}>
        <JsonFallback title="node JSON" data={worker} />
      </Box>
    </>
  );
};

export default WorkerDetailsPanel;
