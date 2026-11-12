import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import App from "./App";
import * as api from "./services/api";

vi.mock("./components/common/TomlEditor", () => ({ default: ({ label, value, onChange, disabled }) =>
  <textarea aria-label={label} value={value} onChange={(event) => onChange(event.target.value)} disabled={disabled} /> }));

vi.mock("./services/api", () => ({
  fetchRuns: vi.fn(),
  fetchNodes: vi.fn(),
  fetchOfflineNodes: vi.fn().mockResolvedValue([]),
  updateOfflineNode: vi.fn(),
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
    window.history.replaceState(null, "", "/");
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
    expect(screen.getByRole("img", { name: "GammaBoard" })).toBeInTheDocument();
    expect(await screen.findByText(/Connected to local/i)).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /System/i })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Performance" })).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
    expect(await screen.findByText(/No runs available/i)).toBeInTheDocument();
    const header = within(screen.getByRole("banner"));
    expect(header.getByText("Connected to local")).toBeVisible();
    expect(header.getByText(/^Last update: \d{2}:\d{2}:\d{2}$/)).toBeVisible();
    for (const name of ["System", "Logs", "Runs"]) {
      fireEvent.click(screen.getByRole("tab", { name, exact: true }));
      expect(header.getByText("Connected to local")).toBeVisible();
      expect(screen.getAllByText(/Connected to/)).toHaveLength(1);
      expect(screen.getAllByText(/Last update:/)).toHaveLength(1);
    }
    expect(screen.queryByText("Run logs")).not.toBeInTheDocument();
  });

  test("a delayed created-run refresh preserves a newly opened management draft", async () => {
    api.fetchSession.mockResolvedValue({ authenticated: true });
    api.createRun.mockResolvedValue({ run_id: 2, run_name: "new-run" });
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "New run" }));
    fireEvent.change(await screen.findByLabelText("Run TOML"), { target: { value: 'name="new-run"' } });
    fireEvent.click(screen.getByRole("button", { name: "Create run" }));
    await waitFor(() => expect(screen.queryByLabelText("Run TOML")).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("tab", { name: "System" }));
    fireEvent.click(await screen.findByRole("button", { name: "Request nodes" }));
    const draft = '[[groups]]\ncount=3\nname_prefix="unsaved"';
    fireEvent.change(await screen.findByLabelText("Node Launch TOML"), { target: { value: draft } });
    const requests = api.fetchRuns.mock.calls.length;
    api.fetchRuns.mockResolvedValue({ items: [{ run_id: 2, run_name: "new-run", kind: "integration" }], nextOffset: null });
    await waitFor(() => expect(api.fetchRuns.mock.calls.length).toBeGreaterThan(requests), { timeout: 6000 });
    await act(async () => {});
    expect(screen.getByText("System").closest('[role="tab"]')).toHaveAttribute("aria-selected", "true");
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

  test("showing children preserves the selected root's panels and disclosure state", async () => {
    let finishChildren;
    const root = { run_id: 1, run_name: "Parent", kind: "integration" };
    api.fetchRuns.mockImplementation(({ includeChildren }) => includeChildren
      ? new Promise((resolve) => { finishChildren = resolve; })
      : Promise.resolve({ items: [root], nextOffset: null }));
    api.fetchRunTasks.mockResolvedValue([{ id: "1", name: "inference", state: "completed", task_kind: "sample" }]);
    api.fetchRunTaskPanels.mockResolvedValue({ source_id: "task", poll_after_ms: null,
      panels: [{ panel_id: "estimate_summary", kind: "key_value", label: "Result" }],
      updates: [{ mode: "replace", panel: { panel_id: "estimate_summary", kind: "key_value", entries: [{ key: "mean", label: "Mean", value: 1.25 }] } }],
    });
    await renderApp();
    const mean = await screen.findByRole("button", { name: "Mean", exact: true });
    const queue = screen.getByRole("button", { name: "Task queue" });
    fireEvent.click(mean);
    fireEvent.click(queue);
    const polls = api.fetchRunTaskPanels.mock.calls.length;
    fireEvent.click(screen.getByRole("switch", { name: "Show child runs" }));
    expect(screen.getByRole("button", { name: "Mean", exact: true })).toBe(mean);
    expect(mean).toHaveAttribute("aria-expanded", "true");
    expect(queue).toHaveAttribute("aria-expanded", "false");
    await act(async () => finishChildren({ items: [root, { run_id: 2, run_name: "Child", parent_run_id: 1 }], nextOffset: null }));
    expect(screen.getByRole("button", { name: "Mean", exact: true })).toBe(mean);
    expect(api.fetchRunTaskPanels).toHaveBeenCalledTimes(polls);
    expect(window.location.hash).toContain("run=1");
  });

  test.each(["integration", "integration_campaign", "parameter_scan", "hyperparameter_tuning"])("only integrations expose queue controls: %s", async (kind) => {
    api.fetchSession.mockResolvedValue({ authenticated: true });
    api.fetchRuns.mockResolvedValue({ items: [{ run_id: 1, run_name: "example", kind }], nextOffset: null });
    api.fetchRunTasks.mockResolvedValue([{ id: "1", name: "execution", task_kind: kind, state: "active" }]);
    await renderApp();
    // A request can start before React finishes loading the queue components.
    // Wait for the rendered workspace before checking its controls.
    await screen.findByRole("tablist", { name: "Task output" });
    expect(api.fetchRunTaskPanels).toHaveBeenCalled();
    if (kind === "integration") {
      expect(screen.getByRole("button", { name: "Task queue", exact: true })).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("button", { name: "Add tasks" })).toBeInTheDocument();
    } else {
      expect(screen.queryByRole("button", { name: "Task queue", exact: true })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Add tasks" })).not.toBeInTheDocument();
    }
  });

});

test("a deep link resolves a paginated run and keeps task output selection across workspaces", async () => {
  vi.clearAllMocks();
  window.history.replaceState(null, "", "/#run=2&task=9007199254740993&taskTab=observables&children=1");
  api.fetchSession.mockResolvedValue({ authenticated: true });
  api.fetchRuns.mockImplementation(async ({ offset = 0 }) => offset
    ? { items: [{ run_id: 2, run_name: "older run", kind: "integration" }], nextOffset: null }
    : { items: [{ run_id: 1, run_name: "new run", kind: "integration" }], nextOffset: 100 });
  api.fetchNodes.mockResolvedValue([]);
  api.fetchServerStatus.mockResolvedValue({ status: "ok" });
  api.fetchNodeLaunchRequests.mockResolvedValue([]);
  api.fetchRunTasks.mockResolvedValue([{ id: "9007199254740993", name: "selected sample", state: "active", task_kind: "sample" }]);
  api.fetchRunTaskPanels.mockResolvedValue({ source_id: "task:2", panels: [{ panel_id: "gammaloop_histogram_bundle", kind: "table", label: "Observables" }, { panel_id: "task_summary", kind: "text", label: "Task summary" }], updates: [{ mode: "replace", panel: { panel_id: "task_summary", kind: "text", text: "Selected task metadata" } }] });
  render(<App />);
  expect(await screen.findByRole("tab", { name: "Observables", selected: true })).toBeInTheDocument();
  expect(api.fetchRuns).toHaveBeenCalledWith(expect.objectContaining({ offset: 100, includeChildren: true }), expect.any(AbortSignal));
  expect(api.fetchRunTaskPanels.mock.calls.every(([run]) => run === 2)).toBe(true);
  fireEvent.click(screen.getByRole("tab", { name: "System", exact: true }));
  expect(await screen.findByRole("tab", { name: "Workers", selected: true })).toBeVisible();
  fireEvent.click(screen.getByRole("tab", { name: "Runs", exact: true }));
  expect(screen.getByRole("tab", { name: "Observables", selected: true })).toBeVisible();
  expect(screen.getByRole("row", { name: "Select task selected sample" })).toHaveClass("Mui-selected");
  expect(window.location.hash).toContain("task=9007199254740993");
  expect(screen.queryByText("Selected task metadata")).not.toBeInTheDocument();
  const polls = api.fetchRunTaskPanels.mock.calls.length;
  fireEvent.click(screen.getByRole("tab", { name: "Summary", exact: true }));
  expect(await screen.findByText("Selected task metadata")).toBeVisible();
  expect(screen.getAllByText("Task summary", { exact: true })).toHaveLength(1);
  expect(api.fetchRunTaskPanels).toHaveBeenCalledTimes(polls + 1);
  expect(api.fetchRunTaskPanels.mock.lastCall[2]).toMatchObject({ section: "summary", phase: "current" });
});
