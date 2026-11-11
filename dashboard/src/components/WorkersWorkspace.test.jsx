import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import WorkersWorkspace from "./WorkersWorkspace";
import { stopIdleNodes, stopAllNodes, unassignAllNodes, fetchOfflineNodes, updateOfflineNode } from "../services/api";

const auth = vi.hoisted(() => ({ authenticated: true }));
vi.mock("../auth/AuthProvider", () => ({ useAuth: () => auth }));
vi.mock("../hooks/useNodeLaunchRequests", () => ({ useNodeLaunchRequests: () => ({ launchRequests: [] }) }));
vi.mock("./runs/TomlActionDialog", () => ({ default: () => null }));
vi.mock("./NodeLaunchQueue", () => ({ default: () => null }));
vi.mock("./WorkerDetailsPanel", () => ({ default: () => null }));
vi.mock("../services/api", () => ({ stopIdleNodes: vi.fn(), stopAllNodes: vi.fn(), unassignAllNodes: vi.fn(), autoRunNodes: vi.fn(), fetchOfflineNodes: vi.fn(), updateOfflineNode: vi.fn() }));

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

test("offline tab replaces live controls with bulk actions and reports partial failures", async () => {
  const offline = [
    { node_name: "old-1", node_uuid: "one", status: "retired", can_resume: true },
    { node_name: "old-2", node_uuid: "two", status: "suspended", can_resume: true },
    { node_name: "queued", node_uuid: "three", status: "starting", can_resume: false, launch_pending: true },
  ];
  fetchOfflineNodes.mockResolvedValue(offline);
  updateOfflineNode.mockResolvedValueOnce({ rows_updated: 1 }).mockRejectedValueOnce(new Error("Node became live"));
  render(<WorkersWorkspace {...props} />);
  fireEvent.click(screen.getByRole("tab", { name: "Offline / suspended" }));
  await screen.findByText("old-1");
  expect(screen.queryByRole("button", { name: "Request nodes" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Stop all nodes" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Resume all" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Release all" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Retire all" }));
  expect(await screen.findByText(/Retired 1 worker.*Skipped 1 unavailable worker.*old-2: Node became live/)).toBeVisible();
  expect(updateOfflineNode.mock.calls).toEqual([[offline[0], "retire"], [offline[1], "retire"]]);
  fireEvent.click(screen.getByRole("tab", { name: "Live nodes" }));
  expect(screen.getByRole("button", { name: "Request nodes" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Resume all" })).not.toBeInTheDocument();
});
