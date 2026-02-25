import { Box, Card, CardContent, Chip, Grid, Typography } from "@mui/material";

const ImplementationSummaryCard = ({ implementation, chipColor = "default", fields, footer, sx }) => (
  <Card sx={sx}>
    <CardContent>
      <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 2 }}>
        <Typography variant="subtitle2" color="text.secondary">
          Implementation
        </Typography>
        <Chip label={implementation || "unknown"} color={chipColor} variant="outlined" />
      </Box>

      <Grid container spacing={2}>
        {(fields || []).map((field) => (
          <Grid key={field.label} item xs={field.xs ?? 12} sm={field.sm ?? 6} md={field.md ?? 4}>
            <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
              {field.label}
            </Typography>
            <Typography variant="h5">{field.value}</Typography>
          </Grid>
        ))}
      </Grid>

      {footer ? <Box sx={{ mt: 2 }}>{footer}</Box> : null}
    </CardContent>
  </Card>
);

export default ImplementationSummaryCard;
