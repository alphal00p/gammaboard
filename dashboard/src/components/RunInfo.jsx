import {
  Box,
  Card,
  CardContent,
  Chip,
  Grid,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import JsonFallback from "./JsonFallback";
import { formatDateTime, formatScientific } from "../utils/formatters";
import { parseScalarTarget } from "../utils/target";
import { deriveObservableImplementation, splitKindConfig, toConfigObject } from "../utils/config";
import { deriveRunLifecycle, formatRunLabel } from "../utils/runs";

const taskKindLabel = (task) => task?.task?.kind ?? "unknown";

const taskTargetLabel = (task) => {
  const raw = Number(task?.task?.nr_samples);
  return Number.isFinite(raw) ? raw.toLocaleString() : "unbounded";
};

const RunInfo = ({ run, tasks = [] }) => {
  if (!run) return null;

  const integrationParams = toConfigObject(run.integration_params);
  const { implementation: evaluatorImplementation } = splitKindConfig(integrationParams.evaluator, "unknown");
  const { implementation: samplerImplementation } = splitKindConfig(integrationParams.sampler_aggregator, "unknown");
  const { implementation: parametrizationImplementation } = splitKindConfig(
    integrationParams.parametrization,
    "unknown",
  );
  const observableImplementation = deriveObservableImplementation(integrationParams.evaluator, null, "unknown");
  const pointSpec = toConfigObject(run.point_spec);
  const hasPointSpec = Number.isInteger(pointSpec?.continuous_dims) && Number.isInteger(pointSpec?.discrete_dims);
  const scalarTarget = parseScalarTarget(run.target);
  const trainingCompleted = Boolean(run.training_completed_at);
  const trainingLabel = trainingCompleted ? "training completed" : "training";
  const lifecycle = deriveRunLifecycle(run);
  const producedSamples = Number(run.nr_produced_samples);
  const completedSamples = Number(run.nr_completed_samples);
  const hasProducedSamples = Number.isFinite(producedSamples);
  const hasCompletedSamples = Number.isFinite(completedSamples);
  const currentTask =
    tasks.find((task) => task.state === "active") ?? tasks.find((task) => task.state === "pending") ?? null;

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
                    current_task
                  </Typography>
                  <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                    {currentTask ? `#${currentTask.sequence_nr} ${taskKindLabel(currentTask)}` : "none"}
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
        <Card sx={{ height: "100%" }}>
          <CardContent>
            <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
              Task Queue
            </Typography>
            {currentTask ? (
              <Box sx={{ mb: 2 }}>
                <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                  current: #{currentTask.sequence_nr} {taskKindLabel(currentTask)} ({currentTask.state})
                </Typography>
                <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                  task_target_samples: {taskTargetLabel(currentTask)}
                </Typography>
                <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                  task_produced_samples: {Number(currentTask.nr_produced_samples || 0).toLocaleString()}
                </Typography>
                <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                  task_completed_samples: {Number(currentTask.nr_completed_samples || 0).toLocaleString()}
                </Typography>
              </Box>
            ) : (
              <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
                No task is currently active or pending.
              </Typography>
            )}
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Seq</TableCell>
                  <TableCell>State</TableCell>
                  <TableCell>Task</TableCell>
                  <TableCell align="right">Target</TableCell>
                  <TableCell align="right">Produced</TableCell>
                  <TableCell align="right">Completed</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {tasks.map((task) => (
                  <TableRow
                    key={task.id}
                    sx={task.state === "active" ? { backgroundColor: "action.hover" } : undefined}
                  >
                    <TableCell>{task.sequence_nr}</TableCell>
                    <TableCell>{task.state}</TableCell>
                    <TableCell>{taskKindLabel(task)}</TableCell>
                    <TableCell align="right">{taskTargetLabel(task)}</TableCell>
                    <TableCell align="right">{Number(task.nr_produced_samples || 0).toLocaleString()}</TableCell>
                    <TableCell align="right">{Number(task.nr_completed_samples || 0).toLocaleString()}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </Box>
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
