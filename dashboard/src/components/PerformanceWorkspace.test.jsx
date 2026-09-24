import { act, fireEvent, render, screen, cleanup } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import PerformanceWorkspace from "./PerformanceWorkspace";
import { useRunPerformancePanels } from "../hooks/useRunPerformancePanels";

vi.mock("../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: false }) }));
vi.mock("../hooks/useRunTasks", () => ({ useRunTasks: () => ({ tasks: [] }) }));
vi.mock("../hooks/useRunPerformancePanels", () => ({ useRunPerformancePanels: vi.fn() }));
vi.mock("./common/RunScopedWorkspace", () => ({ default: ({ children }) => children }));
vi.mock("./runs/QueueTuningPanel", () => ({ default: () => <div>Queue controls</div> }));
vi.mock("./PerformanceGraphs", () => ({ default: () => <div>Historical graphs</div> }));
vi.mock("./panels/PanelCollection", () => ({ default: ({ panelStates }) => <div data-testid="measurements">{JSON.stringify(panelStates)}</div> }));

const props = { runs: [{ run_id: 1 }], workers: [], selectedRun: 1, isConnected: true };
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
  test("keeps diagnostics and queue controls out of the overview and preserves unknown values", () => {
    render(<PerformanceWorkspace {...props} />);
    expect(screen.getByTestId("measurements")).toHaveTextContent("Unavailable");
    expect(screen.getByTestId("measurements")).toHaveTextContent('"value":0');
    expect(screen.getByTestId("measurements")).toHaveTextContent('["Evaluators",80,50]');
    expect(screen.getByTestId("measurements")).toHaveTextContent('["Sampler",0,"Unavailable"]');
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("queue_diagnostics");
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("busy_history");
    expect(screen.queryByText("Queue controls")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(screen.getByTestId("measurements")).toHaveTextContent("queue_diagnostics");
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("performance_overview");
    expect(screen.getByText("Queue controls")).toBeInTheDocument();
    expect(screen.getByTestId("measurements")).not.toHaveTextContent("busy_history");
    fireEvent.click(screen.getByRole("tab", { name: "Graphs" }));
    expect(screen.getByText("Historical graphs")).toBeInTheDocument();
    expect(screen.queryByTestId("measurements")).not.toBeInTheDocument();
    expect(screen.queryByText("Queue controls")).not.toBeInTheDocument();
  });

  test("uses a fixed live window without a dropdown; graphs navigate history separately", () => {
    render(<PerformanceWorkspace {...props} />);
    expect(screen.queryByRole("combobox", { name: "Measurement window" })).not.toBeInTheDocument();
    expect(useRunPerformancePanels).toHaveBeenLastCalledWith(expect.objectContaining({ windowSeconds: 60 }));
    fireEvent.click(screen.getByRole("tab", { name: "Graphs" }));
    expect(screen.getByText("Historical graphs")).toBeInTheDocument();
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
