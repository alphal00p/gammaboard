import { Box, Card, CardContent, Chip, Grid, Typography } from "@mui/material";
import JsonFallback from "./JsonFallback";
import { formatDateTime, formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";
import { deriveObservableImplementation, splitKindConfig, toConfigObject } from "../utils/config";
import { deriveRunLifecycle, formatRunLabel } from "../utils/runs";
import { getCurrentTask, getTaskKindLabel } from "../utils/tasks";

const RunInfo = ({ run, tasks = [] }) => {
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
                started: {formatDateTime(run.started_at)}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                completed: {formatDateTime(run.completed_at)}
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
