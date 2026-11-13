import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import OfflineNodesPanel from "./OfflineNodesPanel";
import { fetchOfflineNodes, updateOfflineNode } from "../services/api";
vi.mock("../auth/AuthProvider", () => ({ useAuth: () => ({authenticated:true,allowLocalNodeSpawn:true}) }));
vi.mock("../services/api", () => ({ fetchOfflineNodes:vi.fn(),updateOfflineNode:vi.fn() }));
const node = {node_name:"old",node_uuid:"identity",status:"offline",restore_requested:true,pool_run_id:3,pool_run_name:"Example",pool_role:"sampler_aggregator",can_start:true,backend:"local"};
beforeEach(()=>{vi.resetAllMocks();fetchOfflineNodes.mockResolvedValue([node,{node_name:"manual",node_uuid:"manual",status:"offline",can_start:false,restore_requested:false}]);});
test("individual Start preserves node identity even without a restoration marker", async()=>{
  const stopped = { ...node, restore_requested: false };
  fetchOfflineNodes.mockResolvedValue([stopped, {node_name:"manual",node_uuid:"manual",status:"offline",can_start:false}]);
  let finish;updateOfflineNode.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  render(<OfflineNodesPanel/>);await screen.findByText("old");
  const row=screen.getByText("old").closest("tr");
  expect(within(row).getByText(/Example #3/)).toBeVisible();
  expect(within(row).getByRole("button",{name:"Cancel restoration"})).toBeDisabled();
  fireEvent.click(within(row).getByRole("button",{name:"Start",exact:true}));
  expect(updateOfflineNode).toHaveBeenCalledWith(stopped,"start");
  expect(within(row).getByRole("button",{name:"Unassign"})).toBeDisabled();
  await act(async()=>finish({request_id:4}));
  expect(screen.getByText("Queued start for old.")).toBeVisible();
  expect(within(screen.getByText("manual").closest("tr")).getByRole("button",{name:"Start",exact:true})).toBeDisabled();
});
test("assignment release failures keep the row usable and explain the error", async()=>{
  updateOfflineNode.mockRejectedValue(new Error("Node became live. Refresh the node list."));
  render(<OfflineNodesPanel/>);await screen.findByText("old");
  fireEvent.click(within(screen.getByText("old").closest("tr")).getByRole("button",{name:"Unassign"}));
  await waitFor(()=>expect(screen.getByRole("alert")).toHaveTextContent("Node became live"));
  expect(updateOfflineNode).toHaveBeenCalledWith(node,"unassign-offline");
});

test("canceling restoration removes its badge and disables repeating it without deleting the worker", async () => {
  fetchOfflineNodes.mockResolvedValue([node]);
  updateOfflineNode.mockResolvedValue({ rows_updated: 1 });
  render(<OfflineNodesPanel />);
  await screen.findByText("old");
  expect(screen.getByText("Saved for restart")).toBeVisible();
  fetchOfflineNodes.mockResolvedValue([{ ...node, restore_requested: false }]);
  fireEvent.click(screen.getByRole("button", { name: "Cancel restoration", exact: true }));
  await waitFor(() => expect(screen.queryByText("Saved for restart")).not.toBeInTheDocument());
  expect(screen.getByText("old")).toBeVisible();
  expect(screen.getByRole("button", { name: "Cancel restoration", exact: true })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Start", exact: true })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Unassign", exact: true })).toBeEnabled();
  expect(screen.queryByRole("button", { name: /Retire|Delete/ })).not.toBeInTheDocument();
  expect(updateOfflineNode).toHaveBeenCalledWith(node, "cancel-restoration");
});
