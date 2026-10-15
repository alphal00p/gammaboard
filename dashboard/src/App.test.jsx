import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  fetchRunPanels: vi.fn(),
  fetchTemplateList: vi.fn(),
  fetchTemplateFile: vi.fn(),
  saveTemplateFile: vi.fn(),
  deleteTemplateFile: vi.fn(),
  fetchRunPerformance: vi.fn(),
  fetchNodeLaunchRequests: vi.fn(),
  fetchServerStatus: vi.fn(),
  shutdownControlProcess: vi.fn(),
  createRun: vi.fn(),
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
    api.fetchRunPanels.mockResolvedValue({ source_id: "run", panels: [], updates: [] });
    api.fetchTemplateList.mockResolvedValue([]);
    api.fetchTemplateFile.mockResolvedValue({ name: "template.toml", toml: "" });
    api.fetchRunPerformance.mockResolvedValue({ source_id: "performance", panels: [], updates: [] });
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
    expect(screen.getByAltText(/GammaBoard/i)).toBeInTheDocument();
    expect(await screen.findByText(/Connected to local/i)).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Management/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Performance/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
    expect(await screen.findByText(/No runs available/i)).toBeInTheDocument();
  });

  test("a delayed created-run refresh preserves a newly opened management draft", async () => {
    api.fetchSession.mockResolvedValue({ authenticated: true });
    api.createRun.mockResolvedValue({ run_id: 2, run_name: "new-run" });
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "New run" }));
    fireEvent.change(await screen.findByLabelText("Run TOML"), { target: { value: 'name="new-run"' } });
    fireEvent.click(screen.getByRole("button", { name: "Create run" }));
    await waitFor(() => expect(screen.queryByLabelText("Run TOML")).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("tab", { name: "Management" }));
    fireEvent.click(await screen.findByRole("button", { name: "Request Nodes" }));
    const draft = '[[groups]]\ncount=3\nname_prefix="unsaved"';
    fireEvent.change(await screen.findByLabelText("Node Launch TOML"), { target: { value: draft } });
    const requests = api.fetchRuns.mock.calls.length;
    api.fetchRuns.mockResolvedValue({ items: [{ run_id: 2, run_name: "new-run", kind: "integration" }], nextOffset: null });
    await waitFor(() => expect(api.fetchRuns.mock.calls.length).toBeGreaterThan(requests), { timeout: 6000 });
    await act(async () => {});
    expect(screen.getByText("Management").closest('[role="tab"]')).toHaveAttribute("aria-selected", "true");
    expect(screen.getByLabelText("Node Launch TOML")).toHaveValue(draft);
  }, 10000);
  test("checks browser access before fetching workspaces and recovers after retry", async () => {
    const message = "Browser origin http://localhost:39491 is not allowed. Restart with --allowed-origin 'http://localhost:39491'";
    api.fetchSession.mockRejectedValueOnce(Object.assign(new Error(message), { status: 403 }));
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    expect(api.fetchRuns).not.toHaveBeenCalled();
    expect(api.fetchRunTaskPanels).not.toHaveBeenCalled();
    expect(screen.queryByRole("tab", { name: "Runs" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("tab", { name: "Runs" })).toBeInTheDocument();
    await waitFor(() => expect(api.fetchRuns).toHaveBeenCalled());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  test.each(["integration", "integration_campaign", "parameter_scan", "hyperparameter_tuning"])("only integrations expose queue controls: %s", async (kind) => {
    api.fetchSession.mockResolvedValue({ authenticated: true });
    api.fetchRuns.mockResolvedValue({ items: [{ run_id: 1, run_name: "example", kind }], nextOffset: null });
    api.fetchRunTasks.mockResolvedValue([{ id: "1", name: "execution", task_kind: kind, state: "active" }]);
    await renderApp();
    await waitFor(() => expect(api.fetchRunTaskPanels).toHaveBeenCalled());
    if (kind === "integration") {
      expect(screen.getByText("Task Queue")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Add tasks" })).toBeInTheDocument();
    } else {
      expect(screen.queryByText("Task Queue")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Add Task" })).not.toBeInTheDocument();
    }
  });

});
