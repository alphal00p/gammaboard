import { Box, Card, CardContent, Grid, Typography } from "@mui/material";
import { Layers as LayersIcon, Analytics as AnalyticsIcon } from "@mui/icons-material";

const AggregatedBatchesPanel = ({ latestAggregated, run }) => {
  const observable = latestAggregated?.aggregated_observable || {};
  const aggregatedBatchesRaw = observable.nr_batches ?? run?.batches_completed ?? 0;
  const aggregatedBatches = Number.isFinite(Number(aggregatedBatchesRaw)) ? Number(aggregatedBatchesRaw) : 0;

  const batchSizeRaw = run?.integration_params?.sampler_aggregator_params?.batch_size;
  const batchSize = Number.isFinite(Number(batchSizeRaw)) ? Number(batchSizeRaw) : 0;

  const aggregatedSamples =
    batchSize > 0 ? aggregatedBatches * batchSize : Number(observable.count ?? observable.nr_samples ?? 0);

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Aggregated
      </Typography>

      <Grid container spacing={2}>
        <Grid item xs={12} sm={6}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1 }}>
                <LayersIcon color="primary" />
                <Typography variant="subtitle2" color="text.secondary">
                  Aggregated Batches
                </Typography>
              </Box>
              <Typography variant="h4" color="primary.main">
                {aggregatedBatches.toLocaleString()}
              </Typography>
            </CardContent>
          </Card>
        </Grid>

        <Grid item xs={12} sm={6}>
          <Card sx={{ height: "100%" }}>
            <CardContent>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1 }}>
                <AnalyticsIcon color="info" />
                <Typography variant="subtitle2" color="text.secondary">
                  Aggregated Samples
                </Typography>
              </Box>
              <Typography variant="h4" color="info.main">
                {aggregatedSamples.toLocaleString()}
              </Typography>
            </CardContent>
          </Card>
        </Grid>
      </Grid>
    </Box>
  );
};

export default AggregatedBatchesPanel;
