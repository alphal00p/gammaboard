import { Suspense, forwardRef, lazy } from "react";
import { Box } from "@mui/material";

const ReactECharts = lazy(() =>
  Promise.all([import("echarts-for-react"), import("../../lib/echarts")]).then(([module]) => ({
    default: module.default,
  })),
);

const LazyChart = forwardRef((props, ref) => (
  <Suspense
    fallback={
      <Box
        sx={{
          width: "100%",
          height: "100%",
          minHeight: 160,
          display: "grid",
          placeItems: "center",
          color: "text.secondary",
          typography: "body2",
        }}
      >
        Loading chart...
      </Box>
    }
  >
    <ReactECharts ref={ref} {...props} />
  </Suspense>
));
LazyChart.displayName = "LazyChart";

export default LazyChart;
