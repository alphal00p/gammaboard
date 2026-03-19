import { Box, Card, CardContent, Chip, Grid, Typography } from "@mui/material";
import JsonFallback from "./JsonFallback";
import { formatDateTime, formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";
import { deriveObservableImplementation, splitKindConfig, toConfigObject } from "../utils/config";
import { deriveRunLifecycle, formatRunLabel } from "../utils/runs";
import { getCurrentTask, getTaskKindLabel } from "../utils/tasks";
import { asArray, asObjectOrNull } from "../utils/collections";

const fmtCount = (value, fallback = "0") =>
  Number.isFinite(Number(value)) ? Number(value).toLocaleString() : fallback;

const formatLifecycleDate = (value, fallback) => formatDateTime(value, fallback);

const findActiveSamplerWorker = (workers) =>
  asArray(workers).find((worker) => worker?.current_role === "sampler_aggregator") ?? null;

const queueRemainingMean = (worker) => {
  const rolling = asObjectOrNull(asObjectOrNull(worker?.sampler_runtime_metrics)?.rolling);
  const metric = asObjectOrNull(rolling?.queue_remaining_ratio);
  const mean = Number(metric?.mean);
  return Number.isFinite(mean) ? mean : null;
};

const queueTarget = (integrationParams) => {
  const params = toConfigObject(integrationParams?.sampler_aggregator_runner_params);
  const value = Number(params?.target_queue_remaining);
  return Number.isFinite(value) ? value : null;
};

const RunInfo = ({ run, tasks = [], workers = [] }) => {
  if (!run) return null;

  const integrationParams = toConfigObject(run.integration_params);
  const { implementation: evaluatorImplementation } = splitKindConfig(integrationParams.evaluator, "unknown");
  const observableImplementation = deriveObservableImplementation(integrationParams.evaluator, null, "unknown");
  const pointSpec = toConfigObject(run.point_spec);
  const hasPointSpec = Number.isInteger(pointSpec?.continuous_dims) && Number.isInteger(pointSpec?.discrete_dims);
  const scalarTarget = parseScalarTarget(run.target);
  const lifecycle = deriveRunLifecycle(run);
  const producedSamples = Number(run.nr_produced_samples);
  const completedSamples = Number(run.nr_completed_samples);
  const hasProducedSamples = Number.isFinite(producedSamples);
  const hasCompletedSamples = Number.isFinite(completedSamples);
  const currentTask = getCurrentTask(tasks);
  const activeSamplerWorker = findActiveSamplerWorker(workers);
  const avgQueueRemaining = queueRemainingMean(activeSamplerWorker);
  const targetQueueRemaining = queueTarget(integrationParams);

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Run Info / Run Spec
      </Typography>

      <Grid container spacing={2}>
        <Grid item xs={12} sm={6} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block" }}
              >
                Status
              </Typography>
              <Chip
                label={lifecycle}
                color={lifecycle === "running" ? "success" : lifecycle === "pausing" ? "warning" : "default"}
                size="medium"
                sx={{ mt: 1, fontWeight: 600 }}
              />
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} sm={6} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block" }}
              >
                Run
              </Typography>
              <Typography variant="body1" component="div" color="primary.main" sx={{ fontWeight: 700 }}>
                {formatRunLabel(run)}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} sm={6} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block", mb: 0.5 }}
              >
                Lifecycle
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                started: {formatLifecycleDate(run.started_at, "not started")}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                completed: {formatLifecycleDate(run.completed_at, "not completed")}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} sm={6} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block", mb: 0.5 }}
              >
                Progress
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                run_tasks: {tasks.length.toLocaleString()}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                produced_samples: {hasProducedSamples ? producedSamples.toLocaleString() : "0"}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                completed_samples: {hasCompletedSamples ? completedSamples.toLocaleString() : "0"}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} sm={6} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block", mb: 0.5 }}
              >
                Queue
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                pending: {fmtCount(run.pending_batches)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                claimed: {fmtCount(run.claimed_batches)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                failed: {fmtCount(run.failed_batches)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                completed: {fmtCount(run.completed_batches)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                avg_queue_remaining: {avgQueueRemaining != null ? avgQueueRemaining.toFixed(4) : "waiting"}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                target_queue_remaining: {targetQueueRemaining != null ? targetQueueRemaining.toFixed(4) : "unset"}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
                Run Spec Summary
              </Typography>
              <Grid container spacing={1.5}>
                <Grid item xs={12} md={6}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    evaluator
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {evaluatorImplementation}
                  </Typography>
                </Grid>
                <Grid item xs={12} md={6}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    observable
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {observableImplementation}
                  </Typography>
                </Grid>
                <Grid item xs={12} md={6}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    point_spec
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {hasPointSpec
                      ? `continuous=${pointSpec.continuous_dims}, discrete=${pointSpec.discrete_dims}`
                      : "n/a"}
                  </Typography>
                </Grid>
                <Grid item xs={12} md={6}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    current_task
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {currentTask ? `#${currentTask.sequence_nr} ${getTaskKindLabel(currentTask)}` : "none"}
                  </Typography>
                </Grid>
                <Grid item xs={12}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    target
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {scalarTarget
                      ? `scalar(${formatScientific(scalarTarget.value, 6)})`
                      : run.target
                        ? "custom json"
                        : "none"}
                  </Typography>
                </Grid>
              </Grid>
            </CardContent>
          </Card>
        </Grid>
      </Grid>

      <Box sx={{ mt: 2 }}>
        <JsonFallback title="integration_params JSON" data={integrationParams} />
      </Box>
      <Box sx={{ mt: 2 }}>
        <JsonFallback title="target JSON" data={run.target} />
      </Box>
    </Box>
  );
};

export default RunInfo;
