import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import RunSelector from "./RunSelector";

test("child options keep their dash and add indentation", () => {
  render(<RunSelector runs={[{run_id:1,run_name:"Campaign"},{run_id:2,run_name:"Child",parent_run_id:1}]} selectedRun={1} />);
  fireEvent.focus(screen.getByRole("combobox", { name: "Select run" }));
  const parent = screen.getByRole("option", { name: /^Campaign/ });
  const child = screen.getByRole("option", { name: /^- Child/ });
  expect(parseFloat(getComputedStyle(child).paddingLeft)).toBeGreaterThan(parseFloat(getComputedStyle(parent).paddingLeft));
});

const runs = [
  { run_id: 1, run_name: "Campaign", lifecycle_state: "running" },
  { run_id: 2, run_name: "Tetrahedron", parent_run_id: 1, lifecycle_state: "completed" },
  { run_id: 3, run_name: "Tetrahedron", lifecycle_state: "paused" },
];

test("run search survives polling, distinguishes duplicate names and selects by ID", () => {
  const change = vi.fn();
  const { rerender } = render(<RunSelector runs={runs} selectedRun={1} onRunChange={change} />);
  const input = screen.getByRole("combobox", { name: "Select run" });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "tetra" } });
  expect(screen.getAllByRole("option")).toHaveLength(2);
  rerender(<RunSelector runs={runs.map((run) => ({ ...run, cpu_hours_including_children: 5 }))} selectedRun={1} onRunChange={change} />);
  expect(input).toHaveValue("tetra");
  fireEvent.change(input, { target: { value: "#2" } });
  expect(screen.getAllByRole("option")).toHaveLength(1);
  fireEvent.click(screen.getByRole("option", { name: /^- Tetrahedron/ }));
  expect(change).toHaveBeenCalledWith(2);
});

test("status search and pagination remain available without selecting free text", () => {
  const change = vi.fn();
  const loadMore = vi.fn();
  render(<RunSelector runs={runs} selectedRun={1} onRunChange={change} hasMoreRuns onLoadMoreRuns={loadMore} />);
  const input = screen.getByRole("combobox", { name: "Select run" });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "paused" } });
  expect(screen.getByRole("option")).toHaveTextContent("#3");
  fireEvent.change(input, { target: { value: "unloaded-run" } });
  expect(screen.getByText(/No matching loaded runs/)).toBeVisible();
  fireEvent.keyDown(input, { key: "Enter" });
  expect(change).not.toHaveBeenCalled();
  fireEvent.blur(input);
  expect(input).toHaveValue("Campaign");
  fireEvent.click(screen.getByRole("button", { name: "Load more runs" }));
  expect(loadMore).toHaveBeenCalledOnce();
});
