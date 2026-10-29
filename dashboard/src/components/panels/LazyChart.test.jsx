import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import LazyChart from "./LazyChart";

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
        };
      }
      componentWillUnmount() {
        events.push("dispose");
      }
      render() {
        return <div data-testid="chart" />;
      }
    },
  };
});

test("clears queued drawing before the child disposes and preserves the export ref", async () => {
  const ref = createRef();
  const { unmount } = render(<LazyChart ref={ref} option={{}} />);
  await screen.findByTestId("chart");
  expect(ref.current.getEchartsInstance()).toBeDefined();
  unmount();
  expect(events).toEqual(["clear", "dispose"]);
  expect(ref.current).toBeNull();
});
