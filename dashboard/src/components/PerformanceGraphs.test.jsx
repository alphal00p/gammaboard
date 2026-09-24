import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import PerformanceGraphs, { mergeHistoryDetail, selectedBounds } from "./PerformanceGraphs";
import { fetchRunPerformanceGraphs } from "../services/api";

const { renderPanels } = vi.hoisted(() => ({ renderPanels: vi.fn() }));
vi.mock("../services/api", () => ({ fetchRunPerformanceGraphs: vi.fn() }));
vi.mock("./panels/PanelCollection", () => ({ default: (props) => {
  renderPanels(props);
  return <button onClick={() => props.onPanelValueChange("busy_history", { zoom: { start: 25, end: 50 } })}>Select history</button>;
} }));
const Harness = ({ initial = null }) => {
  const [selection, setSelection] = useState(initial);
  return <><button onClick={() => setSelection(null)}>All</button>
    <PerformanceGraphs runId={12} selection={selection} onSelectionChange={setSelection} /></>;
};
const full = {
  bounds: [1000, 401000], selection: [1000, 401000], bin_seconds: 2 / 3,
  cadence: [{ intervals: 10, total_seconds: 5, min_seconds: 0.49, max_seconds: 0.51 }, { intervals: 0 }],
  panels: [{ panel_id: "busy_history" }],
  states: [{ panel_id: "busy_history", series: [{ id: "evaluator-compute", points: [
    { x: 1000, y: 10 }, { x: 120000, y: 30 }, { x: 200000, y: 90 }, { x: 401000, y: 0 },
  ] }] }],
};
const detail = { ...full, selection: [101000, 201000], bin_seconds: 1 / 6, states: [
  { panel_id: "busy_history", series: [{ id: "evaluator-compute", points: [{ x: 120000, y: 0 }, { x: 150000, y: 100 }] }] },
] };
beforeEach(() => {
  vi.useFakeTimers();
  fetchRunPerformanceGraphs.mockImplementation((_run, range) => Promise.resolve(range ? detail : full));
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });

test("graph navigation fetches detail, synchronizes ranges, and can restore full history", async () => {
  render(<Harness />);
  await act(async () => vi.advanceTimersByTimeAsync(1));
  expect(fetchRunPerformanceGraphs).toHaveBeenCalledWith(12, null, expect.any(AbortSignal));
  expect(screen.getByText(/Observed report spacing/)).toHaveTextContent("0.5 s average");
  fireEvent.click(screen.getByText("Select history"));
  await act(async () => vi.advanceTimersByTimeAsync(151));
  expect(fetchRunPerformanceGraphs).toHaveBeenLastCalledWith(12, { start: 101000, end: 201000 }, expect.any(AbortSignal));
  const values = renderPanels.mock.lastCall[0].panelValues;
  expect(values.busy_history.zoom).toEqual({ start: 25, end: 50 });
  expect(values.accepted_rate_history.zoom).toEqual(values.busy_history.zoom);
  fireEvent.click(screen.getByText("All"));
  await act(async () => vi.advanceTimersByTimeAsync(1));
  expect(fetchRunPerformanceGraphs).toHaveBeenLastCalledWith(12, null, expect.any(AbortSignal));
});

test("fixed past selection stays anchored while follow mode preserves its duration", () => {
  expect(selectedBounds({ start: 100, end: 200, follow: false }, [0, 2000])).toEqual([100, 200]);
  expect(selectedBounds({ start: 100, end: 200, follow: true }, [0, 2000])).toEqual([1900, 2000]);
  expect(selectedBounds(null, [0, 2000])).toEqual([0, 2000]);
});

test("detail replaces coarse values without drawing across unknown intervals", () => {
  const state = mergeHistoryDetail(full, detail)[0];
  expect(state.x_range).toEqual(full.bounds);
  expect(state.series[0].points).toEqual([
    { x: 1000, y: 10 }, { x: 120000, y: 0, break_before: true },
    { x: 150000, y: 100 }, { x: 401000, y: 0, break_before: true },
  ]);
});

test("a failed refresh keeps historical data visible with a warning", async () => {
  render(<Harness />);
  await act(async () => vi.advanceTimersByTimeAsync(1));
  fetchRunPerformanceGraphs.mockRejectedValue(new Error("History offline"));
  await act(async () => vi.advanceTimersByTimeAsync(5000));
  expect(screen.getByText("Select history")).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("History offline");
});


test("rolling presets constrain the plot and keep full navigation context", async () => {
  render(<Harness initial={{ seconds: 30 }} />);
  await act(async () => vi.advanceTimersByTimeAsync(151));
  expect(fetchRunPerformanceGraphs).toHaveBeenLastCalledWith(12, { start: 371000, end: 401000 }, expect.any(AbortSignal));
  const props = renderPanels.mock.lastCall[0];
  expect(props.panelStates[0].x_range).toEqual(full.bounds);
  expect(props.panelValues.busy_history.zoom).toEqual({ start: 92.5, end: 100 });
  expect(selectedBounds({ seconds: 300 }, [1000, 2000])).toEqual([1000, 2000]);
});


test("vertical zoom preserves a rolling preset", async () => {
  const change = vi.fn();
  render(<PerformanceGraphs runId={12} selection={{ seconds: 30 }} onSelectionChange={change} />);
  await act(async () => vi.advanceTimersByTimeAsync(151));
  act(() => renderPanels.mock.lastCall[0].onPanelValueChange("busy_history", {
    zoom: { start: 92.5, end: 100 }, yZoom: { start: 20, end: 80 },
  }));
  expect(change).not.toHaveBeenCalled();
  expect(renderPanels.mock.lastCall[0].panelValues.busy_history.yZoom).toEqual({ start: 20, end: 80 });
});
