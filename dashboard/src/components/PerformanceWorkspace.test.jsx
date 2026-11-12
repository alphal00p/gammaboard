import { act, fireEvent, render, screen, cleanup } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import PerformanceWorkspace from "./PerformanceWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";
import { updateRunTaskQueueTuning } from "../services/api";

vi.mock("../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: true }) }));
vi.mock("../services/api", () => ({ updateRunTaskQueueTuning: vi.fn() }));
vi.mock("../hooks/useRunPerformancePanels", () => ({ useRunPerformancePanels: vi.fn() }));
vi.mock("./RunInfo", () => ({ default: () => <div>Live queue and batch</div> }));
vi.mock("./PerformanceGraphs", () => ({ default: ({ onSelectionChange }) => <button onClick={() => onSelectionChange({ start: 1000, end: 2000, follow: false })}>Historical graphs</button> }));
vi.mock("./panels/PanelCollection", () => ({ default: ({ panelStates }) => <div data-testid="measurements">{JSON.stringify(panelStates)}</div> }));

const defaults = { target_batch_eval_ms: 2000, max_generation_size: 1024, max_batch_size: 512 };
const props = { runs: [{ run_id: 1, queue_tuning_defaults: defaults }], workers: [], selectedRun: 1, isConnected: true };
const activeTask = { id: "1", name: "train", is_sample: true, state: "active" };
let response;
beforeEach(() => {
  response = {
    panelSpecs: ["performance_overview", "busy_rates", "measurement_window", "queue_diagnostics"].map((panel_id) => ({ panel_id })),
    panelStates: [
      { panel_id: "performance_overview", kind: "key_value", entries: [{ key: "rate", value: null }, { key: "count", value: 0 }] },
      { panel_id: "busy_rates", kind: "table", columns: ["Role", "Compute busy (%)", "I/O active (%)"], rows: [["Evaluators", 80, 50], ["Sampler", 0, null]] },
      { panel_id: "measurement_window", kind: "key_value", entries: [{ key: "window", value: 60 }] },
      { panel_id: "queue_diagnostics", kind: "key_value", entries: [{ key: "pending", value: 4 }] },
      { panel_id: "busy_history", kind: "multi_timeseries", series: [] },
    ], error: null,
  };
  useRunPerformancePanels.mockReturnValue(response);
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });

describe("performance measurements", () => {
  test("tuning follows the active task, resets drafts and isolates saves across task transitions", async () => {
    let finishTrain;
    let finishInference;
    updateRunTaskQueueTuning.mockImplementationOnce(() => new Promise((resolve) => { finishTrain = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { finishInference = resolve; }));
    const inference = { ...activeTask, id: "2", name: "inference", state: "pending" };
    const { rerender } = render(<PerformanceWorkspace {...props} view="tuning" tasks={[activeTask, inference]} />);
    const target = () => screen.getByLabelText("Target Evaluation Time (ms)");
    fireEvent.change(target(), { target: { value: "750" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(updateRunTaskQueueTuning).toHaveBeenLastCalledWith(1, "1", { ...defaults, target_batch_eval_ms: 750 });

    rerender(<PerformanceWorkspace {...props} view="tuning" tasks={[{ ...activeTask, state: "completed" }, { ...inference, state: "active" }]} />);
    expect(screen.getByText("inference", { selector: "strong" })).toBeVisible();
    expect(target()).toHaveValue("2000");
    fireEvent.change(target(), { target: { value: "900" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(updateRunTaskQueueTuning).toHaveBeenLastCalledWith(1, "2", { ...defaults, target_batch_eval_ms: 900 });
    await act(async () => finishTrain());
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(screen.queryByText("Queue tuning updated.")).not.toBeInTheDocument();
    await act(async () => finishInference());
    expect(screen.getByRole("button", { name: "Apply" })).toBeEnabled();
    expect(screen.getByText("Queue tuning updated.")).toBeVisible();
    expect(target()).toHaveValue("900");
  });

  test.each(["pending", "active", "completed", "failed"])("queue tuning is available only for an active sampling task: %s", (state) => {
    const { rerender } = render(<PerformanceWorkspace {...props} view="tuning" tasks={[{ id: "1", name: "Sample", is_sample: true, state }]} />);
    expect(Boolean(screen.queryByText("Live queue and batch"))).toBe(state === "active");
    expect(Boolean(screen.queryByRole("button", { name: "Apply" }))).toBe(state === "active");
    expect(screen.queryByRole("combobox", { name: "Sampling task" })).not.toBeInTheDocument();
    rerender(<PerformanceWorkspace {...props} view="tuning" tasks={[{ id: "1", name: "Sample", is_sample: true, state: "completed" }]} />);
    expect(screen.queryByText("Live queue and batch")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Queue tuning" })).toBeDisabled();
    expect(screen.getByText(/No active sampling task/)).toBeVisible();
  });
  test("shows initial loading but keeps background refreshes quiet", () => {
    vi.useFakeTimers();
    useRunPerformancePanels.mockReturnValue({ ...response, panelSpecs: [], panelStates: [], isLoading: true, isFetching: true });
    const { rerender } = render(<PerformanceWorkspace {...props} />);
    act(() => vi.advanceTimersByTime(200));
    expect(screen.getByRole("status")).toHaveAccessibleName("Loading measurements…");
    useRunPerformancePanels.mockReturnValue({ ...response, isLoading: false, isFetching: false });
    rerender(<PerformanceWorkspace {...props} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    useRunPerformancePanels.mockReturnValue({ ...response, isLoading: false, isFetching: true });
    rerender(<PerformanceWorkspace {...props} />);
    act(() => vi.advanceTimersByTime(200));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.getByTestId("measurements")).toBeInTheDocument();
  });

  test("keeps diagnostics and queue controls out of the overview and preserves unknown values", () => {
    render(<PerformanceWorkspace {...props} tasks={[activeTask]} />);
    expect(screen.getByTestId("measurements")).toHaveTextContent("Unavailable");
    expect(screen.getByTestId("measurements")).toHaveTextContent('"value":0');
    expect(screen.getByTestId("measurements")).toHaveTextContent('["Evaluators",80,50]');
    expect(screen.getByTestId("measurements")).toHaveTextContent('["Sampler",0,"Unavailable"]');
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("queue_diagnostics");
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("busy_history");
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(screen.getByTestId("measurements")).toHaveTextContent("queue_diagnostics");
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("performance_overview");
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Queue tuning" }));
    expect(screen.getByRole("button", { name: "Apply" })).toBeVisible();
    expect(screen.queryByTestId("measurements")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Graphs" }));
    expect(screen.getByText("Historical graphs")).toBeInTheDocument();
    expect(screen.queryByTestId("measurements")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
  });

  test("shares presets and custom graph selections across tabs and resets on run change", () => {
    const { rerender } = render(<PerformanceWorkspace {...props} />);
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: { seconds: 30 } }));
    fireEvent.click(screen.getByRole("button", { name: "5 min" }));
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: { seconds: 300 } }));
    fireEvent.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: { seconds: 300 } }));
    fireEvent.click(screen.getByRole("tab", { name: "Graphs" }));
    fireEvent.click(screen.getByText("Historical graphs"));
    expect(screen.getByRole("button", { name: "Custom" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("tab", { name: "Overview" }));
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: { start: 1000, end: 2000, follow: false } }));
    fireEvent.click(screen.getByRole("button", { name: "All" }));
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: null }));
    rerender(<PerformanceWorkspace {...props} selectedRun={2} />);
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ selection: { seconds: 30 } }));
  });

  test("hides retained values on a failed or missing refresh", () => {
    vi.useFakeTimers();
    const { rerender } = render(<PerformanceWorkspace {...props} />);
    act(() => vi.advanceTimersByTime(10001));
    expect(screen.queryByTestId("measurements")).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("unavailable");
    useRunPerformancePanels.mockReturnValue({ ...response, error: "Refresh failed" });
    rerender(<PerformanceWorkspace {...props} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Refresh failed");
    expect(screen.queryByTestId("measurements")).not.toBeInTheDocument();
  });
});
