import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import LazyChart from "./LazyChart";
import { createTheme, ThemeProvider } from "@mui/material/styles";

const { events } = vi.hoisted(() => ({ events: [] }));

vi.mock("../../lib/echarts", () => ({}));
vi.mock("echarts-for-react", async () => {
  const { Component } = await import("react");
  return {
    default: class MockChart extends Component {
      getEchartsInstance() {
        return {
          isDisposed: () => false,
          clear: () => events.push("clear"),
          getOption: () => ({ legend: [{ selected: { Target: false } }], dataZoom: [{ start: 20, end: 70 }] }),
        };
      }
      componentWillUnmount() {
        events.push("dispose");
      }
      render() {
        return <div data-testid="chart" data-background={this.props.option.backgroundColor} />;
      }
    },
  };
});

test("clears queued drawing before the child disposes and preserves the export ref", async () => {
  const ref = createRef();
  events.length = 0;
  const { unmount } = render(<LazyChart ref={ref} option={{}} />);
  await screen.findByTestId("chart");
  expect(ref.current.getEchartsInstance()).toBeDefined();
  unmount();
  expect(events).toEqual(["clear", "dispose"]);
  expect(ref.current).toBeNull();
});

test("changing theme updates chart options without disposing the chart", async () => {
  events.length = 0;
  const ref = createRef();
  const option = { xAxis: {}, series: [] };
  const light = createTheme();
  const dark = createTheme({ palette: { mode: "dark" } });
  const { rerender } = render(<ThemeProvider theme={light}><LazyChart ref={ref} option={option} /></ThemeProvider>);
  const chart = await screen.findByTestId("chart");
  const instance = ref.current;
  rerender(<ThemeProvider theme={dark}><LazyChart ref={ref} option={option} /></ThemeProvider>);
  expect(chart).toHaveAttribute("data-background", dark.palette.background.paper);
  expect(ref.current).toBe(instance);
  expect(events).toEqual([]);
});
