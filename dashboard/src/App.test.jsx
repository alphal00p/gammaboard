import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import App from "./App";
import { fetchNodes, fetchRuns } from "./services/api";
import * as api from "./services/api";

vi.mock("./services/api", () => ({
  fetchRuns: vi.fn(async () => []),
  fetchNodes: vi.fn(async () => []),
  fetchSession: vi.fn(async () => ({ authenticated: false })),
  login: vi.fn(async () => ({ authenticated: true })),
  logout: vi.fn(async () => ({ authenticated: false })),
  fetchStats: vi.fn(async () => []),
  fetchRunLogPage: vi.fn(async () => ({
    items: [],
    next_before_id: null,
    has_more_older: false,
  })),
  fetchRun: vi.fn(async () => null),
  fetchRunTasks: vi.fn(async () => []),
  fetchRunEvaluatorConfigPanels: vi.fn(async () => ({ source_id: "cfg:evaluator", panels: [], updates: [] })),
  fetchRunSamplerConfigPanels: vi.fn(async () => ({ source_id: "cfg:sampler", panels: [], updates: [] })),
  fetchRunTaskPanels: vi.fn(async () => ({ source_id: "task", panels: [], updates: [] })),
  fetchTemplateList: vi.fn(async () => []),
  fetchTemplateFile: vi.fn(async () => ({ name: "template.toml", toml: "" })),
  fetchEvaluatorPerformanceHistory: vi.fn(async () => ({ source_id: "perf:run:evaluator", panels: [], updates: [] })),
  fetchSamplerPerformanceHistory: vi.fn(async () => ({ source_id: "perf:sampler", panels: [], updates: [] })),
  fetchNodeEvaluatorPerformanceHistory: vi.fn(async () => ({ source_id: "perf:evaluator", panels: [], updates: [] })),
  fetchNodeSamplerPerformanceHistory: vi.fn(async () => ({
    source_id: "perf:node:sampler",
    panels: [],
    updates: [],
  })),
  fetchNodeLaunchRequests: vi.fn(async () => []),
  shutdownControlProcess: vi.fn(async () => ({ shutdown_requested: true })),
}));

/**
 * Basic smoke test for the App component
 *
 * Tests that the main application renders without crashing
 * and contains expected core elements.
 */
describe("App Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
    api.fetchNodeLaunchRequests.mockResolvedValue([]);
    api.shutdownControlProcess.mockResolvedValue({ shutdown_requested: true });
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
    expect(screen.getByRole("tab", { name: /Management/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Performance/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
  });

  test("shows no-runs empty state when run list is empty", async () => {
    await renderApp();
    const emptyMessage = screen.getByText(/No runs available/i);
    expect(emptyMessage).toBeInTheDocument();
  });
});
