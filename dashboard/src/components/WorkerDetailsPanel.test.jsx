import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import WorkerDetailsPanel from "./WorkerDetailsPanel";
import { assignNode, unassignNode } from "../services/api";

vi.mock("../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: true }) }));
vi.mock("../hooks/useNodePanels", () => ({ useNodePanels: () => ({ panelSpecs: [], panelStates: [] }) }));
vi.mock("./panels/PanelCollection", () => ({ default: () => null }));
vi.mock("../services/api", () => ({ assignNode: vi.fn(), unassignNode: vi.fn(), stopNode: vi.fn() }));
afterEach(() => vi.clearAllMocks());

const worker = { node_name: "worker-1" };
const runs = [
  { run_id: 1, run_name: "Completed", lifecycle_state: "completed" },
  { run_id: 2, run_name: "Failed", lifecycle_state: "failed" },
  { run_id: 3, run_name: "Paused", lifecycle_state: "paused" },
];

test("unavailable pools stay visible and unassigned workers cannot be removed", () => {
  render(<WorkerDetailsPanel worker={worker} runs={runs} />);
  expect(screen.getByRole("button", { name: "Assign", exact: true })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Remove from pool" })).toBeDisabled();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Worker pool" }));
  for (const name of ["Completed", "Failed"]) expect(screen.getByRole("option", { name })).toHaveAttribute("aria-disabled", "true");
  expect(screen.getByRole("option", { name: "Paused" })).not.toHaveAttribute("aria-disabled", "true");
});

test("assignment eligibility updates during polling while cleanup stays available", async () => {
  const assigned = { ...worker, pool_run_id: 3, pool_role: "evaluator" };
  const { rerender } = render(<WorkerDetailsPanel worker={assigned} runs={runs} />);
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Assign", exact: true })));
  expect(assignNode).toHaveBeenCalledWith("worker-1", { runId: 3, role: "evaluator" });
  rerender(<WorkerDetailsPanel worker={assigned} runs={runs.map((run) => ({ ...run, lifecycle_state: "completed" }))} />);
  expect(screen.getByRole("button", { name: "Assign", exact: true })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Remove from pool" })).toBeEnabled();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Remove from pool" })));
  expect(unassignNode).toHaveBeenCalledWith("worker-1");
});
