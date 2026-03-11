import { Box, Card, CardContent, Chip, Grid, Typography } from "@mui/material";
import JsonFallback from "./JsonFallback";
import { formatDateTime } from "../utils/formatters";

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

const WorkerStatusPanel = ({ worker }) => {
  if (!worker) return null;

  return (
    <>
      <Card variant="outlined" sx={{ mb: 2 }}>
        <CardContent>
          <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
            Worker Status
          </Typography>
          <Grid container spacing={2}>
            <Grid item xs={12} sm={6} md={3}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                worker_id
              </Typography>
              <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                {worker.worker_id}
              </Typography>
            </Grid>
            <Grid item xs={12} sm={6} md={3}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                role
              </Typography>
              <Typography variant="body2">{worker.role || "unknown"}</Typography>
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
                node
              </Typography>
              <Typography variant="body2">{worker.node_id || "n/a"}</Typography>
            </Grid>
            <Grid item xs={12} sm={6} md={3}>
              <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
                desired_run_id
              </Typography>
              <Typography variant="body2">{worker.desired_run_id ?? "n/a"}</Typography>
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

      <JsonFallback
        title="worker status JSON"
        data={{
          worker_id: worker.worker_id ?? null,
          node_id: worker.node_id ?? null,
          desired_run_id: worker.desired_run_id ?? null,
          role: worker.role ?? null,
          implementation: worker.implementation ?? null,
          version: worker.version ?? null,
          status: worker.status ?? null,
          last_seen: worker.last_seen ?? null,
        }}
      />
    </>
  );
};

export default WorkerStatusPanel;
