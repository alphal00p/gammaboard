import { Card, CardContent, Grid, Typography } from "@mui/material";
import LatexFormula from "../../LatexFormula";

const EvaluatorDetailsCard = ({ minEvalTimePerSampleMs, expectedContinuousDims, observableKind, integralLatex }) => (
  <Card>
    <CardContent>
      <Typography variant="subtitle2" color="text.secondary" sx={{ mb: 1 }}>
        Implementation Details
      </Typography>
      <Grid container spacing={2}>
        <Grid item xs={12} md={4}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            min_eval_time_per_sample_ms
          </Typography>
          <Typography variant="h5">{minEvalTimePerSampleMs ?? "n/a"}</Typography>
        </Grid>
        <Grid item xs={12} md={4}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            expected continuous_dims
          </Typography>
          <Typography variant="h5">{expectedContinuousDims}</Typography>
        </Grid>
        <Grid item xs={12} md={4}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            observable kind
          </Typography>
          <Typography variant="h5">{observableKind ?? "n/a"}</Typography>
        </Grid>
        <Grid item xs={12}>
          <Typography variant="caption" color="text.secondary" sx={{ textTransform: "uppercase" }}>
            Integral
          </Typography>
          <LatexFormula latex={integralLatex} />
        </Grid>
      </Grid>
    </CardContent>
  </Card>
);

export default EvaluatorDetailsCard;
