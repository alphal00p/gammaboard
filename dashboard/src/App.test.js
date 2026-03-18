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
  fetchRunTaskOutput: jest.fn(async () => ({ panels: [], current: [], latest_snapshot_id: null })),
  fetchRunTaskOutputHistory: jest.fn(async () => ({ items: [], latest_snapshot_id: null, reset_required: false })),
  fetchEvaluatorPerformanceHistory: jest.fn(async () => []),
  fetchSamplerPerformanceHistory: jest.fn(async () => []),
  fetchNodeEvaluatorPerformanceHistory: jest.fn(async () => ({ run_id: null, entries: [] })),
  fetchNodeSamplerPerformanceHistory: jest.fn(async () => ({ run_id: null, entries: [] })),
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
