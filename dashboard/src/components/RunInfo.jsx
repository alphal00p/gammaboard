import { Box, Card, CardContent, Grid, Typography, Chip } from "@mui/material";
import JsonFallback from "./JsonFallback";
import { formatDateTime, formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";
import { deriveObservableImplementation, splitKindConfig, toConfigObject } from "../utils/config";
import { formatRunLabel } from "../utils/runs";

const RunInfo = ({ run }) => {
  if (!run) return null;

  const integrationParams = toConfigObject(run.integration_params);
  const { implementation: evaluatorImplementation } = splitKindConfig(integrationParams.evaluator, "unknown");
  const { implementation: samplerImplementation } = splitKindConfig(integrationParams.sampler_aggregator, "unknown");
  const { implementation: parametrizationImplementation } = splitKindConfig(
    integrationParams.parametrization,
    "unknown",
  );
  const samplerRunnerParams = toConfigObject(integrationParams.sampler_aggregator_runner_params);
  const observableImplementation = deriveObservableImplementation(integrationParams.evaluator, null, "unknown");
  const pointSpec = toConfigObject(run.point_spec);
  const hasPointSpec = Number.isInteger(pointSpec?.continuous_dims) && Number.isInteger(pointSpec?.discrete_dims);
  const stopOn = toConfigObject(samplerRunnerParams.stop_on);
  const scalarTarget = parseScalarTarget(run.target);
  const trainingCompleted = Boolean(run.training_completed_at);
  const trainingLabel = trainingCompleted ? "training completed" : "training";

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
                label={run.run_status || "unknown"}
                color={run.run_status === "running" ? "success" : "default"}
                size="medium"
                sx={{ mt: 1, fontWeight: 600 }}
              />
              <Chip
                label={trainingLabel}
                color={trainingCompleted ? "info" : "warning"}
                size="small"
                sx={{ mt: 1, fontWeight: 600, ml: 1 }}
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
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                training_completed_at: {formatDateTime(run.training_completed_at)}
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
                completed_batches: {(run.batches_completed ?? 0).toLocaleString()}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                queued_batches: {(run.total_batches ?? 0).toLocaleString()}
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
                    sampler_aggregator
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {samplerImplementation}
                  </Typography>
                </Grid>
                <Grid item xs={12} md={6}>
                  <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                    parametrization
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {parametrizationImplementation}
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
                    auto-stop
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {stopOn.kind === "samples_at_least" && Number.isFinite(Number(stopOn.samples))
                      ? `samples_at_least(${Number(stopOn.samples).toLocaleString()})`
                      : "disabled"}
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
