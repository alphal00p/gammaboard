import { Card, CardContent, Grid, Typography } from "@mui/material";

const SamplerDetailsCard = ({ fields }) => (
  <Card>
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
        Implementation Details
      </Typography>
      <Grid container spacing={2}>
        {(fields || []).map((field) => (
          <Grid key={field.label} item xs={12} md={4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              {field.label}
            </Typography>
            <Typography variant="h5">{field.value ?? "n/a"}</Typography>
          </Grid>
        ))}
      </Grid>
    </CardContent>
  </Card>
);

export default SamplerDetailsCard;
