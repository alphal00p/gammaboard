import { Suspense, forwardRef, lazy, useCallback, useLayoutEffect, useMemo, useRef } from "react";
import { Box, useTheme } from "@mui/material";
import { withChartTheme } from "./chartTheme";
import { useWorkspaceActive } from "../common/WorkspaceTabs";

const ReactECharts = lazy(() =>
  Promise.all([import("echarts-for-react"), import("../../lib/echarts")]).then(([module]) => ({
    default: module.default,
  })),
);

const LazyChart = forwardRef((props, ref) => {
  const { palette } = useTheme();
  const active = useWorkspaceActive();
  const wasActive = useRef(active);
  const chartRef = useRef(null);
  const previous = useRef({ option: props.option, palette });
  const option = useMemo(() => {
    // Some panels replace all options on data changes. A theme-only update must
    // retain the live legend/zoom state, including controls not stored by a panel.
    const themeOnly = previous.current.palette !== palette && previous.current.option === props.option;
    const navigation = themeOnly ? chartRef.current?.getEchartsInstance()?.getOption() : null;
    return withChartTheme(props.option, palette, navigation);
  }, [props.option, palette]);
  useLayoutEffect(() => { previous.current = { option: props.option, palette }; });
  const captureChart = useCallback((chart) => {
    chartRef.current = chart;
    if (typeof ref === "function") ref(chart);
    else if (ref) ref.current = chart;
  }, [ref]);
  useLayoutEffect(() => {
    if (active && !wasActive.current) chartRef.current?.getEchartsInstance()?.resize();
    wasActive.current = active;
  }, [active]);
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
      <ReactECharts ref={captureChart} {...props} option={option} autoResize={active} />
    </Suspense>
  );
});
LazyChart.displayName = "LazyChart";

export default LazyChart;
