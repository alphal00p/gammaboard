import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import CampaignChildren from "./CampaignChildren";
import { useCampaign } from "../../hooks/useCampaign";
import { applyCampaignEdit, fetchRunDefinition, previewCampaignEdit } from "../../services/api";
vi.mock("../../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated:true }) }));
vi.mock("../../hooks/useCampaign", () => ({ useCampaign:vi.fn() }));
vi.mock("../../services/api", () => ({ applyCampaignEdit:vi.fn(), previewCampaignEdit:vi.fn(), fetchRunDefinition:vi.fn(), fetchRuns:vi.fn().mockResolvedValue({items:[]}) }));
vi.mock("../common/TomlEditor", () => ({default: ({label,value,onChange}) => <textarea aria-label={label} value={value} onChange={(e) => onChange(e.target.value)} />}));
const children=[{run_id:2,label:"A",status:"running",failed:false},{run_id:3,label:"B",status:"failed",failed:true}];
const getRandomValues = globalThis.crypto.getRandomValues.bind(globalThis.crypto);
beforeEach(() => {
  vi.clearAllMocks();
  useCampaign.mockReturnValue({data:{state:"active",children,task_names:[{name:"extra",count:1}],operations:[]},refresh:vi.fn()});
  applyCampaignEdit.mockResolvedValue({updated:[2]});
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });
test("bulk add requires compatibility preview and confirmation, excluding failed children",async () => {
  const preview={children:[{...children[0],applicable:true},{...children[1],applicable:false,reason:"child run has failed"}]};
  previewCampaignEdit.mockResolvedValue(preview);
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button",{name:"Add task",exact:true}));
  fireEvent.change(await screen.findByLabelText("Task TOML"),{target:{value:"[task]\nname='extra'"}});
  expect(screen.getByRole("button",{name:"Confirm add task"})).toBeDisabled();
  fireEvent.click(screen.getByRole("button",{name:"Check compatibility"}));
  await screen.findByText("B: child run has failed");
  expect(applyCampaignEdit).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:"Confirm add task"}));
  await waitFor(() => expect(applyCampaignEdit).toHaveBeenCalledWith(1,"4",expect.objectContaining({action:"add_task",children:[2],expected:preview,request_id:expect.any(String)})));
});
test("create loads the selected child's current definition as an editable draft",async () => {
  fetchRunDefinition.mockResolvedValue({toml:"name='source'\n# current tasks"});
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button",{name:"Create",exact:true}));
  fireEvent.mouseDown(screen.getByRole("combobox",{name:"Start from child (optional)"}));
  fireEvent.click(await screen.findByRole("option",{name:/A/}));
  await waitFor(() => expect(screen.getByLabelText("Run TOML")).toHaveValue("name='source'\n# current tasks"));
  expect(fetchRunDefinition).toHaveBeenCalledWith(2);
  expect(screen.getByLabelText("Child label")).toHaveValue("A-copy");
  expect(applyCampaignEdit).not.toHaveBeenCalled();
});
test("changing the task invalidates a previously approved compatibility preview",async () => {
  previewCampaignEdit.mockResolvedValue({children:[{...children[0],applicable:true}]});
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button",{name:"Add task",exact:true}));
  await screen.findByLabelText("Task TOML");
  fireEvent.click(screen.getByRole("button",{name:"Check compatibility"}));
  await waitFor(() => expect(screen.getByRole("button",{name:"Confirm add task"})).toBeEnabled());
  fireEvent.change(screen.getByLabelText("Task TOML"),{target:{value:"changed"}});
  expect(screen.getByRole("button",{name:"Confirm add task"})).toBeDisabled();
});

test.each([
  ["Create", "create", "create child"],
  ["Add task", "add_task", "add task"],
  ["Remove task", "remove_task", "remove task"],
])("%s checks and confirms without crypto.randomUUID", async (button, action, confirmation) => {
  vi.stubGlobal("crypto", { getRandomValues });
  const preview = { children: children.map((child) => ({ ...child, applicable: !child.failed })) };
  previewCampaignEdit.mockResolvedValue(preview);
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button", { name: button, exact: true }));
  if (action === "remove_task") {
    fireEvent.mouseDown(screen.getByLabelText("Task name"));
    fireEvent.click(await screen.findByRole("option", { name: /extra/ }));
  } else {
    fireEvent.change(await screen.findByLabelText(action === "create" ? "Run TOML" : "Task TOML"), { target: { value: "name='example'" } });
    if (action === "create") fireEvent.change(screen.getByLabelText("Child label"), { target: { value: "example" } });
  }
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const confirm = screen.getByRole("button", { name: `Confirm ${confirmation}` });
  await waitFor(() => expect(confirm).toBeEnabled());
  fireEvent.click(confirm);
  await waitFor(() => expect(applyCampaignEdit).toHaveBeenCalledWith(1, "4", expect.objectContaining({
    action, expected: preview, request_id: expect.stringMatching(/^[0-9a-f]{32}$/),
  })));
});

test("a failed request-ID generation invalidates an earlier successful preview", async () => {
  const random = vi.fn(getRandomValues);
  vi.stubGlobal("crypto", { getRandomValues: random });
  previewCampaignEdit.mockResolvedValue({ children: [{ ...children[0], applicable: true }] });
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button", { name: "Add task", exact: true }));
  await screen.findByLabelText("Task TOML");
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const confirm = screen.getByRole("button", { name: "Confirm add task" });
  await waitFor(() => expect(confirm).toBeEnabled());
  random.mockImplementationOnce(() => { throw new Error("Random generator unavailable"); });
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  await screen.findByText("Random generator unavailable");
  expect(confirm).toBeDisabled();
  fireEvent.click(confirm);
  expect(applyCampaignEdit).not.toHaveBeenCalled();
});

test("confirmation retries reuse the request ID on plain HTTP", async () => {
  vi.stubGlobal("crypto", { getRandomValues });
  previewCampaignEdit.mockResolvedValue({ children: [{ ...children[0], applicable: true }] });
  applyCampaignEdit.mockRejectedValueOnce(new Error("Connection interrupted"));
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button", { name: "Add task", exact: true }));
  await screen.findByLabelText("Task TOML");
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const confirm = screen.getByRole("button", { name: "Confirm add task" });
  await waitFor(() => expect(confirm).toBeEnabled());
  fireEvent.click(confirm);
  await screen.findByText("Connection interrupted");
  fireEvent.click(confirm);
  await waitFor(() => expect(applyCampaignEdit).toHaveBeenCalledTimes(2));
  const firstId = applyCampaignEdit.mock.calls[0][2].request_id;
  expect(firstId).toMatch(/^[0-9a-f]{32}$/);
  expect(applyCampaignEdit.mock.calls[1][2].request_id).toBe(firstId);
});

test("changing targets generates a new request ID without crypto.randomUUID", async () => {
  vi.stubGlobal("crypto", { getRandomValues });
  const available = children.map((child) => ({ ...child, failed: false, status: "running", applicable: true }));
  useCampaign.mockReturnValue({ data: { state: "active", children: available }, refresh: vi.fn() });
  previewCampaignEdit.mockResolvedValue({ children: available });
  applyCampaignEdit.mockRejectedValueOnce(new Error("Connection interrupted"));
  render(<CampaignChildren runId={1} taskId="4" />);
  fireEvent.click(screen.getByRole("button", { name: "Add task", exact: true }));
  await screen.findByLabelText("Task TOML");
  fireEvent.click(screen.getByRole("button", { name: "Check compatibility" }));
  const confirm = screen.getByRole("button", { name: "Confirm add task" });
  await waitFor(() => expect(confirm).toBeEnabled());
  fireEvent.click(confirm);
  await screen.findByText("Connection interrupted");
  const targets = screen.getByRole("combobox", { name: "Target children" });
  fireEvent.mouseDown(targets);
  fireEvent.click(await screen.findByRole("option", { name: /B/ }));
  fireEvent.keyDown(targets, { key: "Escape" });
  fireEvent.click(confirm);
  await waitFor(() => expect(applyCampaignEdit).toHaveBeenCalledTimes(2));
  const next = applyCampaignEdit.mock.calls[1][2];
  expect(next.children).toEqual([2]);
  expect(next.request_id).toMatch(/^[0-9a-f]{32}$/);
  expect(next.request_id).not.toBe(applyCampaignEdit.mock.calls[0][2].request_id);
});
