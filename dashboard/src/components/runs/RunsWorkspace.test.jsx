import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import * as api from "../../services/api";
import { copyToClipboard } from "../../utils/clipboard";
import RunsWorkspace from "./RunsWorkspace";

const fixtures = vi.hoisted(() => ({ tasks: [], loadTemplate: vi.fn(), saveTemplate: vi.fn(), templates: [] }));
vi.mock("../../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: true }) }));
vi.mock("../../hooks/useRunTasks", () => ({ useRunTasks: () => ({ tasks: fixtures.tasks }) }));
vi.mock("../../hooks/useTemplates", () => ({ useTemplates: () => ({ templates: fixtures.templates, load: fixtures.loadTemplate, save: fixtures.saveTemplate, remove: vi.fn() }) }));
vi.mock("../../utils/clipboard", () => ({ copyToClipboard: vi.fn() }));
vi.mock("../common/RunScopedWorkspace", () => ({ default: ({ children, headerActions }) => <>{headerActions}{children}</> }));
vi.mock("../RunInfo", () => ({ default: () => null }));
vi.mock("../TaskOutputPanel", () => ({ default: () => null }));
vi.mock("../../services/api", async (original) => ({
  ...await original(), deleteRun: vi.fn(), deleteRunTask: vi.fn(), fetchRunDefinition: vi.fn(), fetchTaskDefinition: vi.fn(),
  createRun: vi.fn(), addRunTasks: vi.fn(), editRunTask: vi.fn(),
}));

const runs = [
  { run_id: 1, run_name: "parent", kind: "integration_campaign" },
  { run_id: 2, run_name: "child", parent_run_id: 1, kind: "integration" },
  { run_id: 3, run_name: "ordinary", kind: "integration" },
];
const task = { id: "9007199254740993", name: "train", state: "pending", task_kind: "sample", sequence_nr: 1 };
const taskToml = '[task]\nname="train"\nkind="sample"';
const renderRun = (id, props = {}) => render(<RunsWorkspace runs={runs} selectedRun={id} {...props} />);
const openRun = async () => { const button = await screen.findByRole("button", { name: "Open run definition" }); await act(async () => fireEvent.click(button)); };
const openTask = async () => { const button = await screen.findByRole("button", { name: /^(Edit|Open) task train$/ }); await act(async () => fireEvent.click(button)); };

beforeEach(() => {
  vi.resetAllMocks(); fixtures.tasks = []; fixtures.templates = []; window.localStorage.clear();
  fixtures.saveTemplate.mockImplementation(async (name) => { fixtures.templates.push(name); return { name }; });
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

describe("definition editors", () => {
  test.each([1, 3])("run %s opens its original definition and duplicates the edited draft", async (id) => {
    fixtures.templates = ["old.toml"];
    window.localStorage.setItem("dialogs.create_run.selected_template", "old.toml");
    const toml = 'name = "source"\nkind = "integration"';
    const draft = 'name = "my-edited-copy"\nkind = "integration"';
    api.fetchRunDefinition.mockResolvedValueOnce({ toml });
    api.createRun.mockResolvedValueOnce({ run_id: 4, run_name: "my-edited-copy" });
    const onRunCreated = vi.fn();
    renderRun(id, { onRunCreated });
    await openRun();
    const input = await screen.findByLabelText("Run TOML");
    expect(input).toHaveValue(toml);
    expect(api.fetchRunDefinition.mock.calls[0].slice(0, 2)).toEqual([id, false]);
    expect(fixtures.loadTemplate).not.toHaveBeenCalled();
    expect(screen.getByText(/External files are not copied or verified/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
    fireEvent.change(input, { target: { value: draft } });
    fireEvent.click(screen.getByRole("button", { name: "Create duplicate run" }));
    await waitFor(() => expect(api.createRun).toHaveBeenCalledWith(draft, { duplicate: true }));
    await waitFor(() => expect(onRunCreated).toHaveBeenCalledWith(4));
  });

  test("managed children allow draft export and standalone run creation", async () => {
    fixtures.tasks = [task];
    api.fetchTaskDefinition.mockResolvedValueOnce({ toml: taskToml });
    api.fetchRunDefinition.mockResolvedValueOnce({ toml: 'name = "child"' });
    api.createRun.mockResolvedValueOnce({ run_id: 4, run_name: "child-copy" });
    renderRun(2);
    expect(await screen.findByText(/Workers are allocated by the parent/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add tasks" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete run" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete task train" })).toBeDisabled();
    await openTask();
    expect(await screen.findByLabelText("Task TOML")).not.toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Duplicate task" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy TOML" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await openRun();
    expect(await screen.findByLabelText("Run TOML")).toHaveValue('name = "child"');
    fireEvent.click(screen.getByRole("button", { name: "Create standalone run" }));
    await waitFor(() => expect(api.createRun).toHaveBeenCalledWith('name = "child"', { duplicate: true }));
  });

  test("task duplication uses unsaved edits and preserves bigint IDs when loading", async () => {
    fixtures.tasks = [task];
    api.fetchTaskDefinition.mockResolvedValueOnce({ toml: taskToml });
    api.addRunTasks.mockResolvedValueOnce([{ id: "another", name: "train-copy" }]);
    renderRun(3); await openTask();
    const input = await screen.findByLabelText("Task TOML");
    expect(input).toHaveValue(taskToml);
    expect(api.fetchTaskDefinition.mock.calls[0].slice(0, 3)).toEqual([3, task.id, false]);
    const draft = `${taskToml}\nstop_condition = { max_samples = 123 }`;
    fireEvent.change(input, { target: { value: draft } });
    expect(screen.queryByRole("button", { name: "Create duplicate run" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Duplicate task" }));
    await waitFor(() => expect(api.addRunTasks).toHaveBeenCalledWith(3, draft, { duplicate: true }));
    expect(api.editRunTask).not.toHaveBeenCalled();
  });

  test("pending saves keep the original concurrency check even when a template replaces the draft", async () => {
    fixtures.tasks = [task];
    fixtures.templates = ["sample.toml"];
    const replacement = '[task]\nname="train"\nkind="sample"\n# from template';
    fixtures.loadTemplate.mockResolvedValueOnce(replacement);
    api.fetchTaskDefinition.mockResolvedValueOnce({ toml: taskToml });
    api.editRunTask.mockRejectedValueOnce(new Error("Task started; reopen the editor"));
    renderRun(3); await openTask();
    await screen.findByLabelText("Task TOML");
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "Template" }));
    fireEvent.click(await screen.findByRole("option", { name: "sample.toml" }));
    await waitFor(() => expect(screen.getByLabelText("Task TOML")).toHaveValue(replacement));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(api.editRunTask).toHaveBeenCalledWith(3, task.id, replacement, taskToml));
    expect(await screen.findByText("Task started; reopen the editor")).toBeInTheDocument();
    expect(screen.getByLabelText("Task TOML")).toHaveValue(replacement);
  });

  test.each(["active", "completed", "failed"])("%s tasks open editable drafts without allowing in-place saves or deletion", async (state) => {
    fixtures.tasks = [{ ...task, state }];
    api.fetchTaskDefinition.mockResolvedValueOnce({ toml: taskToml });
    renderRun(3);
    expect(await screen.findByRole("button", { name: "Delete task train" })).toBeDisabled();
    await openTask();
    expect(await screen.findByLabelText("Task TOML")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Duplicate task" })).toBeEnabled();
    expect(screen.getByText(/Changes can only be saved as a new task/)).toBeInTheDocument();
  });

  test("a task starting while its editor is open disables saving without discarding the draft", async () => {
    fixtures.tasks = [task];
    api.fetchTaskDefinition.mockResolvedValueOnce({ toml: taskToml });
    const view = renderRun(3); await openTask();
    fireEvent.change(await screen.findByLabelText("Task TOML"), { target: { value: `${taskToml}\n# draft` } });
    fixtures.tasks = [{ ...task, state: "active" }];
    view.rerender(<RunsWorkspace runs={runs} selectedRun={3} />);
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    expect(screen.getByLabelText("Task TOML")).toHaveValue(`${taskToml}\n# draft`);
  });

  test("copy and download export the exact current draft, including incomplete TOML", async () => {
    api.fetchRunDefinition.mockResolvedValueOnce({ toml: 'name = "ordinary"' });
    const createObjectURL = vi.fn(() => "blob:draft");
    vi.stubGlobal("URL", class extends URL { static createObjectURL = createObjectURL; static revokeObjectURL = vi.fn(); });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    renderRun(3); await openRun();
    const draft = 'name = "changed"\n# work in progress\n[evaluator';
    fireEvent.change(await screen.findByLabelText("Run TOML"), { target: { value: draft } });
    fireEvent.click(screen.getByRole("button", { name: "Copy TOML" }));
    await screen.findByText("TOML copied.");
    await waitFor(() => expect(copyToClipboard).toHaveBeenCalledWith(draft));
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    const blob = createObjectURL.mock.calls[0][0];
    const text = await new Promise((resolve) => {
      const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.readAsText(blob);
    });
    expect(text).toBe(draft);
    expect(api.createRun).not.toHaveBeenCalled();
  });

  test.each(["New run", "Add tasks"])("%s keeps templates and exports alongside creation", async (action) => {
    renderRun(3); fireEvent.click(await screen.findByRole("button", { name: action }));
    expect(await screen.findByRole("combobox", { name: "Template" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy TOML" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download" })).toBeInTheDocument();
    const value = action === "New run" ? 'name="new"' : taskToml;
    fireEvent.change(screen.getByLabelText(action === "New run" ? "Run TOML" : "Task TOML"), { target: { value } });
    fireEvent.click(screen.getByRole("button", { name: "Save as Template" }));
    fireEvent.change(await screen.findByLabelText("Template file name"), { target: { value: "my-template.toml" } });
    fireEvent.click(screen.getByRole("button", { name: "Save", exact: true }));
    await waitFor(() => expect(fixtures.saveTemplate).toHaveBeenCalledWith("my-template.toml", value));
  });

  test("navigation cancels a pending definition read", async () => {
    let resolve;
    api.fetchRunDefinition.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    const view = renderRun(3); await openRun();
    view.rerender(<RunsWorkspace runs={runs} selectedRun={1} />);
    await act(async () => resolve({ toml: 'name="old-selection"' }));
    expect(screen.queryByLabelText("Run TOML")).not.toBeInTheDocument();
  });
});

test("separate delete button keeps deletion busy and errors visible until dismissed", async () => {
  vi.spyOn(window, "confirm").mockReturnValue(true);
  let fail;
  api.deleteRun.mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; }));
  const setSelectedRun = vi.fn();
  renderRun(1, { setSelectedRun });
  fireEvent.click(await screen.findByRole("button", { name: "Delete run" }));
  expect(screen.getByRole("button", { name: "Open run definition" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Assign / Resume" })).toBeDisabled();
  expect(screen.getByText(/Large histories can take several minutes/)).toBeInTheDocument();
  vi.useFakeTimers();
  await act(async () => { fail(new Error("Database could not finish deletion")); });
  await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
  expect(screen.getByText("Database could not finish deletion")).toBeInTheDocument();
  expect(setSelectedRun).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(screen.queryByText("Database could not finish deletion")).not.toBeInTheDocument();
});
