import { Box, Card, CardContent, Grid, Typography, Chip } from "@mui/material";
import { InfoOutlined as InfoOutlinedIcon } from "@mui/icons-material";
import JsonFallback from "./JsonFallback";
import { formatDateTime, formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";

const RunInfo = ({ run }) => {
  if (!run) return null;

  const integrationParams = run.integration_params || {};
  const evaluatorImplementation = integrationParams.evaluator_implementation || "unknown";
  const samplerImplementation = integrationParams.sampler_aggregator_implementation || "unknown";
  const observableImplementation =
    run.observable_implementation || integrationParams.observable_implementation || "unknown";
  const pointSpec = run.point_spec || integrationParams.point_spec || null;
  const scalarTarget = parseScalarTarget(run.target);
  const trainingCompleted = Boolean(run.training_completed_at);
  const trainingLabel = trainingCompleted ? "training completed" : "training";
  let pointSpecText = "not exposed by /runs/:id";
  if (pointSpec) {
    try {
      pointSpecText = JSON.stringify(pointSpec);
    } catch {
      pointSpecText = "failed to serialize point_spec";
    }
  }

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
                {run.run_name ? `${run.run_name} (#${run.run_id})` : `#${run.run_id}`}
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
                processed_batches_total: {(run.batches_completed ?? 0).toLocaleString()}
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                queue_batches_current: {(run.total_batches ?? 0).toLocaleString()}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1 }}>
                <InfoOutlinedIcon color="disabled" />
                <Typography variant="subtitle2" color="text.secondary">
                  Run Spec (Generic)
                </Typography>
              </Box>
              <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
                evaluator: {evaluatorImplementation}
              </Typography>
              <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
                sampler_aggregator: {samplerImplementation}
              </Typography>
              <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
                observable: {observableImplementation}
              </Typography>
              <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace", mt: 1 }}>
                point_spec: {pointSpecText}
              </Typography>
              <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace", mt: 0.5 }}>
                target:{" "}
                {scalarTarget
                  ? `scalar(${formatScientific(scalarTarget.value, 6)})`
                  : run.target
                    ? "custom json"
                    : "none"}
              </Typography>
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
