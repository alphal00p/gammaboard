import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import App from "./App";
import * as api from "./services/api";

vi.mock("./services/api", () => ({
  fetchRuns: vi.fn(),
  fetchNodes: vi.fn(),
  fetchSession: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
  fetchRuntimeLogPage: vi.fn(),
  fetchRunTasks: vi.fn(),
  fetchRunTaskPanels: vi.fn(),
  fetchTemplateList: vi.fn(),
  fetchTemplateFile: vi.fn(),
  saveTemplateFile: vi.fn(),
  deleteTemplateFile: vi.fn(),
  fetchEvaluatorPerformanceHistory: vi.fn(),
  fetchSamplerPerformanceHistory: vi.fn(),
  fetchNodeLaunchRequests: vi.fn(),
  fetchServerStatus: vi.fn(),
  shutdownControlProcess: vi.fn(),
}));

describe("App Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.fetchRuns.mockResolvedValue({ items: [], nextOffset: null });
    api.fetchNodes.mockResolvedValue([]);
    api.fetchSession.mockResolvedValue({ authenticated: false });
    api.login.mockResolvedValue({ authenticated: true });
    api.logout.mockResolvedValue({ authenticated: false });
    api.fetchRuntimeLogPage.mockResolvedValue({
      items: [],
      next_before_id: null,
      has_more_older: false,
    });
    api.fetchRunTasks.mockResolvedValue([]);
    api.fetchRunTaskPanels.mockResolvedValue({ source_id: "task", panels: [], updates: [] });
    api.fetchTemplateList.mockResolvedValue([]);
    api.fetchTemplateFile.mockResolvedValue({ name: "template.toml", toml: "" });
    api.fetchEvaluatorPerformanceHistory.mockResolvedValue({
      source_id: "perf:run:evaluator",
      panels: [],
      updates: [],
    });
    api.fetchSamplerPerformanceHistory.mockResolvedValue({ source_id: "perf:sampler", panels: [], updates: [] });
    api.fetchNodeLaunchRequests.mockResolvedValue([]);
    api.fetchServerStatus.mockResolvedValue({ status: "ok", database: "connected", server_name: "local" });
    api.shutdownControlProcess.mockResolvedValue({ shutdown_requested: true });
  });

  const renderApp = async () => {
    await act(async () => {
      render(<App />);
    });
    await waitFor(() => {
      expect(api.fetchRuns).toHaveBeenCalled();
      expect(api.fetchNodes).toHaveBeenCalled();
    });
  };

  test("renders the empty runs view and primary navigation", async () => {
    await renderApp();
    expect(screen.getByAltText(/Gammaboard/i)).toBeInTheDocument();
    expect(await screen.findByText(/Connected to local/i)).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Management/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Performance/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
    expect(await screen.findByText(/No runs available/i)).toBeInTheDocument();
  });
});
