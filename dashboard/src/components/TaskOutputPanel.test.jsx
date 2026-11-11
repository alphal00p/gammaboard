import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import TaskOutputPanel from "./TaskOutputPanel";
import { fetchRunTaskPanels } from "../services/api";

vi.mock("../services/api", async (original) => ({ ...await original(), fetchRunTaskPanels: vi.fn() }));
vi.mock("./panels/LazyChart", () => ({
  default: ({ option }) => <div data-testid="chart">{JSON.stringify(option.series)}</div>,
}));

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

const valueSeries = (name = "value") => screen.getAllByTestId("chart")
  .flatMap((element) => JSON.parse(element.textContent)).find((series) => series.name === name);

test.each([
  ["gammaloop_histogram_bundle", "Observables"],
  ["campaign_histograms", "Combined Observables"],
])("%s merges immediately and retains the view through task-output polling", async (panelId, label) => {
  const histogram = {
    sample_count: 10,
    bins: [
      { x_min: 0, x_max: 1, sum_weights: 10, sum_weights_squared: 20 },
      { x_min: 1, x_max: 2, sum_weights: 20, sum_weights_squared: 80 },
      { x_min: 2, x_max: 3, sum_weights: 30, sum_weights_squared: 180 },
    ],
  };
  const response = (scale = 1, incremental = false) => ({
    source_id: "task:1", cursor: "1:0", poll_after_ms: 2000,
    panels: incremental ? [] : [{ panel_id: panelId, kind: "table", label, width: "full" }],
    updates: [{ mode: "replace", panel: {
      panel_id: panelId, kind: "table", columns: ["Observable"], rows: [["energy"]], row_keys: ["energy"],
      payload: {
        expands_to: { kind: "histogram", source: "selected_row" }, actions: { upload_bundle: true },
        histograms: { energy: { ...histogram, bins: histogram.bins.map((bin) => ({
          ...bin, sum_weights: bin.sum_weights * scale, sum_weights_squared: bin.sum_weights_squared * scale ** 2,
        })) } },
      },
    } }],
  });
  let finishInitial;
  let finishRefresh;
  fetchRunTaskPanels.mockResolvedValue(response(2)).mockImplementationOnce(() => new Promise((resolve) => { finishInitial = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { finishRefresh = resolve; }));
  function Output() {
    const [view,setView] = useState("observables");
    return <TaskOutputPanel runId={1} task={{ id: "1", name: "sample", state: "active" }} view={view} onViewChange={setView} />;
  }
  render(<Output />);
  await act(async () => vi.advanceTimersByTime(200));
  expect(screen.getByRole("status")).toHaveAccessibleName("Loading task output…");
  await act(async () => finishInitial(response()));
  fireEvent.click(screen.getByRole("tab", { name: "Observables", exact: true }));
  expect(valueSeries().data).toEqual([[0, 1], [1, 1], [1, 2], [2, 2], [2, 3], [3, 3]]);

  // Change the view while a poll with the old preferences is in flight.
  await act(async () => vi.advanceTimersByTime(2200));
  const input = screen.getByRole("spinbutton", { name: "Merge adjacent bins", exact: true });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "2" } });
  // Do not blur or press Enter: typing and the number-input arrows apply directly.
  expect(valueSeries().data).toEqual([[0, 3], [2, 3], [2, 3], [3, 3]]);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();

  fireEvent.change(input, { target: { value: "" } });
  expect(valueSeries().data).toEqual([[0, 3], [2, 3], [2, 3], [3, 3]]);
  fireEvent.change(input, { target: { value: "3" } });
  expect(valueSeries().data).toEqual([[0, 6], [3, 6]]);
  await act(async () => finishRefresh(response(2)));
  expect(input).toHaveValue(3);
  expect(valueSeries().data).toEqual([[0, 12], [3, 12]]);

  await act(async () => fireEvent.change(screen.getByLabelText("Upload histogram bundle"), { target: { files: [{
    name: "reference.json", text: async () => JSON.stringify({ histograms: { energy: histogram } }),
  }] } }));
  expect(valueSeries("reference.json: energy").data).toEqual([[0, 6], [3, 6]]);
  fireEvent.change(input, { target: { value: "2" } });
  expect(valueSeries("reference.json: energy").data).toEqual([[0, 3], [2, 3], [2, 3], [3, 3]]);
  expect(valueSeries().data).toEqual([[0, 6], [2, 6], [2, 6], [3, 6]]);
  expect(JSON.parse(localStorage.getItem("gammaboard.panel-values.v1.task:1"))[panelId]
    .histogram_view.rebin_factors['["energy",null,null]']).toBe(2);

  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Diagnostics", exact: true })));
  expect(screen.queryByTestId("chart")).not.toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Observables", exact: true })));
  expect(screen.getAllByRole("spinbutton", { name: "Merge adjacent bins", exact: true })).toHaveLength(1);
  expect(screen.getByRole("spinbutton", { name: "Merge adjacent bins", exact: true })).toHaveValue(2);
  expect(valueSeries("reference.json: energy").data).toEqual([[0, 3], [2, 3], [2, 3], [3, 3]]);

  fireEvent.click(screen.getByRole("button", { name: "Reset view", exact: true }));
  expect(valueSeries().data).toEqual([[0, 2], [1, 2], [1, 4], [2, 4], [2, 6], [3, 6]]);
  expect(valueSeries("reference.json: energy").data).toEqual([[0, 1], [1, 1], [1, 2], [2, 2], [2, 3], [3, 3]]);
  expect(histogram.bins.map((bin) => bin.sum_weights)).toEqual([10, 20, 30]);
});

test("task output routes diagnostics and strata while retaining unknown result panels", async () => {
  const groups = { estimate_summary: "Estimate", gammaloop_evaluation_timing: "Timing", max_weight_points: "Extreme points",
    rsd_history: "Primary RSD", strata: "Stratum history", stratum_results: "Stratum values", task_summary: "Task metadata", future_scan_plot: "Future scan result" };
  fetchRunTaskPanels.mockResolvedValue({ source_id: "grouping", poll_after_ms: null,
    panels: Object.entries(groups).map(([panel_id, label]) => ({ panel_id, label, kind: "text" })),
    updates: Object.entries(groups).map(([panel_id, text]) => ({ mode: "replace", panel: { panel_id, kind: "text", text: `${text} data` } })),
  });
  render(<TaskOutputPanel runId={1} task={{ id: "9", name: "sample", state: "completed" }} />);
  await act(async () => {});
  expect(screen.getByText("Estimate data")).toBeVisible();
  expect(screen.getByText("Primary RSD data")).toBeVisible();
  expect(screen.queryByText("Task metadata data")).not.toBeInTheDocument();
  expect(screen.getByText("Future scan result data")).toBeVisible();
  expect(screen.queryByText("Extreme points data")).not.toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Diagnostics" })));
  for (const label of ["Timing", "Extreme points"]) expect(screen.getByText(`${label} data`)).toBeVisible();
  expect(screen.queryByText("Estimate data")).not.toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("tab", { name: "Stratum output" })));
  expect(screen.getByText("Stratum history data")).toBeVisible();
  expect(screen.getByText("Stratum values data")).toBeVisible();
  expect(fetchRunTaskPanels).toHaveBeenCalledTimes(3);
});

test("stratum tabs require output, and a retained stratum selection falls back to Result when absent", async () => {
  const task = { id: "1", name: "sample", state: "active" };
  const result = { panel_id: "result", kind: "text", text: "Result data" };
  const panelSpecs = [{ panel_id: "result", kind: "text", label: "Result" },
    { panel_id: "strata", kind: "strata", label: "Strata" },
    { panel_id: "stratum_results", kind: "table", label: "Stratum results" }];
  const tree = (states) => <TaskOutputPanel task={task} view="strata" output={{ panelSpecs, panelStates: [result, ...states] }} />;
  const { rerender } = render(tree([]));
  expect(screen.queryByRole("tab", { name: "Stratum output" })).not.toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Result" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByText("Result data")).toBeVisible();
  expect(screen.getAllByText("Result", { exact: true })).toHaveLength(2); // Tab and panel, no collection heading.
  await act(async () => rerender(tree([{ panel_id: "stratum_results", kind: "table", columns: ["Stratum"], rows: [["0"]] }])));
  expect(screen.getByRole("tab", { name: "Stratum output" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByText("Stratum results")).toBeVisible();
  await act(async () => rerender(tree([{ panel_id: "stratum_results", kind: "table", rows: [] }, { panel_id: "strata", kind: "strata", frames: [] }])));
  expect(screen.queryByRole("tab", { name: "Stratum output" })).not.toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Result" })).toHaveAttribute("aria-selected", "true");
});
