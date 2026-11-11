import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import WorkersWorkspace from "./WorkersWorkspace";
import { stopIdleNodes, stopAllNodes, unassignAllNodes } from "../services/api";

const auth = vi.hoisted(() => ({ authenticated: true }));
vi.mock("../auth/AuthProvider", () => ({ useAuth: () => auth }));
vi.mock("../hooks/useNodeLaunchRequests", () => ({ useNodeLaunchRequests: () => ({ launchRequests: [] }) }));
vi.mock("./runs/TomlActionDialog", () => ({ default: () => null }));
vi.mock("./NodeLaunchQueue", () => ({ default: () => null }));
vi.mock("./WorkerDetailsPanel", () => ({ default: () => null }));
vi.mock("../services/api", () => ({ stopIdleNodes: vi.fn(), stopAllNodes: vi.fn(), unassignAllNodes: vi.fn(), autoRunNodes: vi.fn() }));

beforeEach(() => { vi.clearAllMocks(); auth.authenticated = true; });
const idle = { node_name: "idle", status: "idle" };
const props = { runs: [], isConnected: true, workers: [idle, { node_name: "working", status: "active", current_run_id: 1, current_role: "evaluator" }] };

test("stop idle uses the atomic endpoint and displays its actual affected count", async () => {
  let finish;
  stopIdleNodes.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  render(<WorkersWorkspace {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Stop Idle nodes" }));
  expect(stopIdleNodes).toHaveBeenCalledOnce();
  expect(stopAllNodes).not.toHaveBeenCalled();
  expect(unassignAllNodes).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Stop Idle nodes" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Unassign all" })).toBeDisabled();
  await act(async () => finish({ rows_updated: 1 }));
  expect(screen.getByText("Requested shutdown for 1 idle node.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Stop Idle nodes" })).toBeEnabled();
});

test("pending and pooled nodes are excluded; errors leave the action usable", async () => {
  const { rerender } = render(<WorkersWorkspace {...props} workers={[
    { ...idle, node_name: "starting", desired_run_id: 1, desired_role: "evaluator" },
    { ...idle, node_name: "pooled", pool_run_id: 1, pool_role: "evaluator" },
    props.workers[1],
  ]} />);
  expect(screen.getByRole("button", { name: "Stop Idle nodes" })).toBeDisabled();
  rerender(<WorkersWorkspace {...props} />);
  stopIdleNodes.mockRejectedValueOnce(new Error("Stop request failed"));
  fireEvent.click(screen.getByRole("button", { name: "Stop Idle nodes" }));
  expect(await screen.findByText("Stop request failed")).toBeVisible();
  expect(screen.getByRole("button", { name: "Stop Idle nodes" })).toBeEnabled();
  auth.authenticated = false;
  rerender(<WorkersWorkspace {...props} />);
  expect(screen.queryByRole("button", { name: "Stop Idle nodes" })).not.toBeInTheDocument();
});
