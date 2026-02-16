import { Box, Card, CardContent, Grid, Typography, Chip } from "@mui/material";
import { InfoOutlined as InfoOutlinedIcon } from "@mui/icons-material";

const RunInfo = ({ run }) => {
  if (!run) return null;

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Run Details
      </Typography>

      <Grid container spacing={2}>
        <Grid item xs={12} md={3}>
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
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} md={3}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ textTransform: "uppercase", display: "block" }}
              >
                Run ID
              </Typography>
              <Typography variant="h4" component="div" color="primary.main">
                #{run.run_id}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} md={6}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1 }}>
                <InfoOutlinedIcon color="disabled" />
                <Typography variant="subtitle2" color="text.secondary">
                  Reserved For Additional Run Metadata
                </Typography>
              </Box>
              <Typography variant="body2" color="text.secondary">
                This section is intentionally left with space for upcoming run configuration and diagnostics.
              </Typography>
            </CardContent>
          </Card>
        </Grid>
      </Grid>
    </Box>
  );
};

export default RunInfo;
