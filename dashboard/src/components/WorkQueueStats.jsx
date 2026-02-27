import { Box, Card, CardContent, Grid, Typography, Chip } from "@mui/material";
import {
  Queue as QueueIcon,
  PlayArrow as PlayArrowIcon,
  CheckCircle as CheckCircleIcon,
  Error as ErrorIcon,
  Timer as TimerIcon,
} from "@mui/icons-material";

const WorkQueueStats = ({ stats }) => {
  const defaultStatuses = ["pending", "claimed", "completed", "failed"];

  const statsByStatus = new Map((stats || []).map((stat) => [(stat.status || "").toLowerCase(), stat]));

  const orderedStats = defaultStatuses.map((status) => {
    const existing = statsByStatus.get(status);
    return (
      existing || {
        status,
        batch_count: 0,
        total_samples: 0,
        avg_sample_time_ms: null,
      }
    );
  });

  const getStatusIcon = (status) => {
    switch (status.toLowerCase()) {
      case "pending":
        return QueueIcon;
      case "claimed":
        return PlayArrowIcon;
      case "completed":
        return CheckCircleIcon;
      case "failed":
        return ErrorIcon;
      default:
        return QueueIcon;
    }
  };

  const getStatusColor = (status) => {
    switch (status.toLowerCase()) {
      case "pending":
        return "default";
      case "claimed":
        return "info";
      case "completed":
        return "success";
      case "failed":
        return "error";
      default:
        return "default";
    }
  };

  return (
    <Box sx={{ mb: 3 }}>
      <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", mb: 1 }}>
        <Typography variant="h6">Work Queue (Pending → Claimed → Completed)</Typography>
      </Box>

      <Grid container spacing={2}>
        {orderedStats.map((stat) => {
          const StatusIcon = getStatusIcon(stat.status);
          const statusColor = getStatusColor(stat.status);

          return (
            <Grid item xs={12} sm={6} md={4} lg={3} key={stat.status}>
              <Card
                sx={{
                  height: "100%",
                  transition: "transform 0.2s, box-shadow 0.2s",
                  "&:hover": {
                    transform: "translateY(-2px)",
                    boxShadow: 3,
                  },
                }}
              >
                <CardContent>
                  <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
                    <StatusIcon sx={{ mr: 1, color: `${statusColor}.main` }} />
                    <Chip
                      label={stat.status}
                      color={statusColor}
                      size="small"
                      sx={{ textTransform: "uppercase", fontWeight: 600 }}
                    />
                  </Box>

                  <Typography variant="h3" component="div" sx={{ mb: 1, fontWeight: 700 }}>
                    {stat.batch_count || 0}
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
                    {stat.batch_count === 1 ? "batch" : "batches"}
                  </Typography>

                  <Box sx={{ pt: 1, borderTop: 1, borderColor: "divider" }}>
                    <Box sx={{ display: "flex", alignItems: "center", mb: 0.5 }}>
                      <Typography variant="caption" color="text.secondary" sx={{ flexGrow: 1 }}>
                        Total samples
                      </Typography>
                      <Typography variant="caption" sx={{ fontWeight: 600 }}>
                        {stat.total_samples?.toLocaleString() || 0}
                      </Typography>
                    </Box>

                    {stat.avg_sample_time_ms != null && (
                      <Box sx={{ display: "flex", alignItems: "center" }}>
                        <TimerIcon sx={{ fontSize: 12, mr: 0.5, color: "text.secondary" }} />
                        <Typography variant="caption" color="text.secondary" sx={{ flexGrow: 1 }}>
                          Avg time/sample
                        </Typography>
                        <Typography variant="caption" sx={{ fontWeight: 600 }}>
                          {stat.avg_sample_time_ms.toFixed(1)}ms
                        </Typography>
                      </Box>
                    )}
                  </Box>
                </CardContent>
              </Card>
            </Grid>
          );
        })}
      </Grid>
    </Box>
  );
};

export default WorkQueueStats;
