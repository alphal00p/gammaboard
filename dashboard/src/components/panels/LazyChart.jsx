import { Suspense, forwardRef, lazy, useCallback, useLayoutEffect, useRef } from "react";
import { Box } from "@mui/material";

const ReactECharts = lazy(() =>
  Promise.all([import("echarts-for-react"), import("../../lib/echarts")]).then(([module]) => ({
    default: module.default,
  })),
);

const LazyChart = forwardRef((props, ref) => {
  const chartRef = useRef(null);
  const captureChart = useCallback((chart) => {
    chartRef.current = chart;
    if (typeof ref === "function") ref(chart);
    else if (ref) ref.current = chart;
  }, [ref]);
  useLayoutEffect(() => () => {
    const chart = chartRef.current?.getEchartsInstance();
    // Clear before the child disposes its canvas: this invalidates pending
    // repaint frames that would otherwise access the disposed painter.
    if (chart && !chart.isDisposed()) chart.clear();
  }, []);
  return (
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
      <ReactECharts ref={captureChart} {...props} />
    </Suspense>
  );
});
LazyChart.displayName = "LazyChart";

export default LazyChart;
