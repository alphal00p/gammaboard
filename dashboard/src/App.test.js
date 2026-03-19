import { render, screen } from "@testing-library/react";
import App from "./App";

jest.mock("./services/api", () => ({
  fetchRuns: jest.fn(async () => []),
  fetchNodes: jest.fn(async () => []),
  fetchStats: jest.fn(async () => []),
  fetchRunLogPage: jest.fn(async () => ({
    items: [],
    next_before_id: null,
    has_more_older: false,
  })),
  fetchRun: jest.fn(async () => null),
  fetchRunTasks: jest.fn(async () => []),
  fetchRunEvaluatorConfigPanels: jest.fn(async () => ({ source_id: "cfg:evaluator", panels: [], updates: [] })),
  fetchRunSamplerConfigPanels: jest.fn(async () => ({ source_id: "cfg:sampler", panels: [], updates: [] })),
  fetchRunTaskPanels: jest.fn(async () => ({ source_id: "task", panels: [], updates: [] })),
  fetchSamplerPerformanceHistory: jest.fn(async () => ({ source_id: "perf:sampler", panels: [], updates: [] })),
  fetchNodeEvaluatorPerformanceHistory: jest.fn(async () => ({ source_id: "perf:evaluator", panels: [], updates: [] })),
  fetchNodeSamplerPerformanceHistory: jest.fn(async () => ({
    source_id: "perf:node:sampler",
    panels: [],
    updates: [],
  })),
}));

/**
 * Basic smoke test for the App component
 *
 * Tests that the main application renders without crashing
 * and contains expected core elements.
 */
describe("App Component", () => {
  test("renders Gammaboard logo", () => {
    render(<App />);
    const logoElement = screen.getByAltText(/Gammaboard/i);
    expect(logoElement).toBeInTheDocument();
  });

  test("renders connection status component", () => {
    render(<App />);
    // ConnectionStatus should show "Disconnected" initially
    const statusElement = screen.getByText(/Disconnected/i);
    expect(statusElement).toBeInTheDocument();
  });

  test("renders mode tabs", () => {
    render(<App />);
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Nodes/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Performance/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
  });

  test("shows no-runs empty state when run list is empty", () => {
    render(<App />);
    const emptyMessage = screen.getByText(/No runs available/i);
    expect(emptyMessage).toBeInTheDocument();
  });
});
