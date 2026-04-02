import { act, render, screen, waitFor } from "@testing-library/react";
import App from "./App";
import { fetchNodes, fetchRuns } from "./services/api";
import * as api from "./services/api";

jest.mock("./services/api", () => ({
  fetchRuns: jest.fn(async () => []),
  fetchNodes: jest.fn(async () => []),
  fetchSession: jest.fn(async () => ({ authenticated: false })),
  login: jest.fn(async () => ({ authenticated: true })),
  logout: jest.fn(async () => ({ authenticated: false })),
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
  fetchTemplateList: jest.fn(async () => []),
  fetchTemplateFile: jest.fn(async () => ({ name: "template.toml", toml: "" })),
  fetchEvaluatorPerformanceHistory: jest.fn(async () => ({ source_id: "perf:run:evaluator", panels: [], updates: [] })),
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
  beforeEach(() => {
    jest.clearAllMocks();
    api.fetchRuns.mockResolvedValue([]);
    api.fetchNodes.mockResolvedValue([]);
    api.fetchSession.mockResolvedValue({ authenticated: false });
    api.login.mockResolvedValue({ authenticated: true });
    api.logout.mockResolvedValue({ authenticated: false });
    api.fetchStats.mockResolvedValue([]);
    api.fetchRunLogPage.mockResolvedValue({
      items: [],
      next_before_id: null,
      has_more_older: false,
    });
    api.fetchRun.mockResolvedValue(null);
    api.fetchRunTasks.mockResolvedValue([]);
    api.fetchRunEvaluatorConfigPanels.mockResolvedValue({ source_id: "cfg:evaluator", panels: [], updates: [] });
    api.fetchRunSamplerConfigPanels.mockResolvedValue({ source_id: "cfg:sampler", panels: [], updates: [] });
    api.fetchRunTaskPanels.mockResolvedValue({ source_id: "task", panels: [], updates: [] });
    api.fetchTemplateList.mockResolvedValue([]);
    api.fetchTemplateFile.mockResolvedValue({ name: "template.toml", toml: "" });
    api.fetchEvaluatorPerformanceHistory.mockResolvedValue({
      source_id: "perf:run:evaluator",
      panels: [],
      updates: [],
    });
    api.fetchSamplerPerformanceHistory.mockResolvedValue({ source_id: "perf:sampler", panels: [], updates: [] });
    api.fetchNodeEvaluatorPerformanceHistory.mockResolvedValue({
      source_id: "perf:evaluator",
      panels: [],
      updates: [],
    });
    api.fetchNodeSamplerPerformanceHistory.mockResolvedValue({
      source_id: "perf:node:sampler",
      panels: [],
      updates: [],
    });
  });

  const renderApp = async () => {
    await act(async () => {
      render(<App />);
    });
    await waitFor(() => {
      expect(fetchRuns).toHaveBeenCalled();
      expect(fetchNodes).toHaveBeenCalled();
    });
  };

  test("renders Gammaboard logo", async () => {
    await renderApp();
    const logoElement = screen.getByAltText(/Gammaboard/i);
    expect(logoElement).toBeInTheDocument();
  });

  test("renders connection status component", async () => {
    await renderApp();
    const statusElement = screen.getByText(/Connected/i);
    expect(statusElement).toBeInTheDocument();
  });

  test("renders mode tabs", async () => {
    await renderApp();
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Nodes/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Performance/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
  });

  test("shows no-runs empty state when run list is empty", async () => {
    await renderApp();
    const emptyMessage = screen.getByText(/No runs available/i);
    expect(emptyMessage).toBeInTheDocument();
  });
});
