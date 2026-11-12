import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import RunSelector from "./RunSelector";
import { fetchRuns } from "../services/api";

vi.mock("../services/api", () => ({ fetchRuns: vi.fn() }));
const runs = [{run_id:1,run_name:"Campaign"},{run_id:2,run_name:"Child",parent_run_id:1}];
beforeEach(() => { vi.clearAllMocks(); fetchRuns.mockResolvedValue({ items: [], nextOffset: null }); });
const open = () => { const input = screen.getByRole("combobox", { name: "Select run" }); fireEvent.focus(input); return input; };

test("child options retain indentation and identify their parent", () => {
  render(<RunSelector runs={runs} selectedRun={1} />);
  open();
  const parent = screen.getByRole("option", { name: /^Campaign/ });
  const child = screen.getByRole("option", { name: /^- Child/ });
  expect(parseFloat(getComputedStyle(child).paddingLeft)).toBeGreaterThan(parseFloat(getComputedStyle(parent).paddingLeft));
  expect(screen.getByText("Parent: Campaign #1")).toBeVisible();
  fireEvent.click(parent);
  expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
});

test("typing survives polling and blur; only selecting a result changes the run", async () => {
  const change = vi.fn();
  const oldChild = { run_id:5,run_name:"Old child",parent_run_id:4,parent_run_name:"Older campaign" };
  fetchRuns.mockResolvedValue({ items: [oldChild], nextOffset: null });
  const { rerender } = render(<RunSelector runs={runs} selectedRun={1} showChildRuns onRunChange={change} />);
  const input = open();
  fireEvent.change(input, { target: { value: "Older" } });
  expect(await screen.findByText("Parent: Older campaign #4")).toBeVisible();
  expect(fetchRuns).toHaveBeenCalledWith({ search:"Older",includeChildren:true }, expect.any(AbortSignal));
  rerender(<RunSelector runs={[...runs]} selectedRun={1} showChildRuns onRunChange={change} />);
  expect(input).toHaveValue("Older");
  expect(change).not.toHaveBeenCalled();
  fireEvent.blur(input);
  expect(input).toHaveValue("Campaign");
  fireEvent.focus(input);
  expect(input).toHaveValue("Older");
  fireEvent.click(await screen.findByRole("option", { name:/^- Old child/ }));
  expect(change).toHaveBeenCalledExactlyOnceWith(5);
  rerender(<RunSelector runs={[...runs, oldChild]} selectedRun={5} showChildRuns onRunChange={change} />);
  expect(input).toHaveValue("- Old child");
});

test("search pagination remains in the dropdown without selecting the load action", async () => {
  const change = vi.fn();
  fetchRuns.mockImplementation(async ({ offset }) => offset
    ? { items:[{run_id:4,run_name:"Old second"}],nextOffset:null }
    : { items:[{run_id:3,run_name:"Old first"}],nextOffset:100 });
  render(<RunSelector runs={runs} selectedRun={1} onRunChange={change} />);
  fireEvent.change(open(), { target: { value: "Old" } });
  fireEvent.click(await screen.findByRole("option", { name: "Load more runs" }));
  expect(await screen.findByRole("option", { name:/Old second/ })).toBeVisible();
  expect(screen.getByRole("option", { name:/Old first/ })).toBeVisible();
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("combobox")).toHaveValue("Old");
});

test("late results from a previous query cannot overwrite a newer query", async () => {
  let resolveOld;
  fetchRuns.mockImplementation(({ search }) => search === "first" ? new Promise((resolve) => { resolveOld=resolve; })
    : Promise.resolve({items:[{run_id:6,run_name:"second result"}],nextOffset:null}));
  render(<RunSelector runs={runs} selectedRun={1} />);
  const input = open();
  fireEvent.change(input, { target:{ value:"first" } });
  await waitFor(() => expect(resolveOld).toBeTypeOf("function"));
  fireEvent.change(input, { target:{ value:"second" } });
  expect(await screen.findByRole("option", { name:/second result/ })).toBeVisible();
  resolveOld({ items:[{run_id:7,run_name:"first result"}],nextOffset:null });
  await waitFor(() => expect(screen.queryByRole("option", {name:/first result/})).not.toBeInTheDocument());
});
