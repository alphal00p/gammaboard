import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import CampaignChildren from "./CampaignChildren";
import { applyCampaignEdit, fetchCampaign, previewCampaignEdit } from "../../services/api";

vi.mock("../../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: true }) }));
vi.mock("../../services/api", () => ({
  fetchCampaign: vi.fn(), applyCampaignEdit: vi.fn(), previewCampaignEdit: vi.fn(),
  fetchRunDefinition: vi.fn(), fetchRuns: vi.fn(),
}));
vi.mock("../common/TomlEditor", () => ({ default: ({ label, value, onChange }) =>
  <textarea aria-label={label} value={value} onChange={(event) => onChange(event.target.value)} /> }));

const children = [{ run_id: 2, label: "A", status: "running", failed: false }];
const data = (taskNames) => ({ state: "active", children, task_names: taskNames, operations: [] });

beforeEach(() => {
  vi.resetAllMocks();
  previewCampaignEdit.mockResolvedValue({ children: [{ ...children[0], applicable: true }] });
  applyCampaignEdit.mockResolvedValue({ updated: [2] });
});

test("adding and removing reload the task menu before another edit can start", async () => {
  let finishAddRefresh, finishRemoveRefresh;
  fetchCampaign.mockResolvedValueOnce(data([]))
    .mockImplementationOnce(() => new Promise((resolve) => { finishAddRefresh = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { finishRemoveRefresh = resolve; }));
  render(<CampaignChildren runId={1} taskId="4" />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Add task", exact: true })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: "Add task", exact: true }));
  fireEvent.change(await screen.findByLabelText("Task TOML"), { target: { value: "[task]\nname='new-task'" } });
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const add = screen.getByRole("button", { name: "Confirm add task" });
  await waitFor(() => expect(add).toBeEnabled());
  fireEvent.click(add);
  await waitFor(() => expect(fetchCampaign).toHaveBeenCalledTimes(2));
  await act(async () => {});
  expect(screen.getByRole("button", { name: "Remove task", exact: true, hidden: true })).toBeDisabled();
  expect(screen.getByRole("dialog", { name: "Add task" })).toBeInTheDocument();
  expect(add).toBeDisabled();

  await act(async () => finishAddRefresh(data([{ name: "new-task", count: 1 }])));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "Remove task", exact: true }));
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Task name" }));
  fireEvent.click(await screen.findByRole("option", { name: "new-task — 1/1 children" }));
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const remove = screen.getByRole("button", { name: "Confirm remove task" });
  await waitFor(() => expect(remove).toBeEnabled());
  fireEvent.click(remove);
  await waitFor(() => expect(fetchCampaign).toHaveBeenCalledTimes(3));
  await act(async () => {});
  expect(screen.getByRole("button", { name: "Add task", exact: true, hidden: true })).toBeDisabled();
  expect(screen.getByRole("dialog", { name: "Remove task" })).toBeInTheDocument();
  expect(remove).toBeDisabled();

  await act(async () => finishRemoveRefresh(data([])));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "Remove task", exact: true }));
  expect(screen.getByRole("combobox", { name: "Task name" })).not.toHaveTextContent("new-task");
  expect(screen.getByRole("combobox", { name: "Task name" }).parentElement.querySelector("input")).toHaveValue("");
  expect(applyCampaignEdit).toHaveBeenCalledTimes(2);
  expect(applyCampaignEdit.mock.calls[1][2]).toMatchObject({ action: "remove_task", task_name: "new-task" });
});
