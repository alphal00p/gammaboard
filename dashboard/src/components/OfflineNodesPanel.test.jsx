import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import OfflineNodesPanel from "./OfflineNodesPanel";
import { fetchOfflineNodes, updateOfflineNode } from "../services/api";
vi.mock("../auth/AuthProvider", () => ({ useAuth: () => ({authenticated:true,allowLocalNodeSpawn:true}) }));
vi.mock("../services/api", () => ({ fetchOfflineNodes:vi.fn(),updateOfflineNode:vi.fn() }));
const node = {node_name:"old",node_uuid:"identity",status:"suspended",pool_run_id:3,pool_run_name:"Example",pool_role:"sampler_aggregator",can_resume:true,backend:"local"};
beforeEach(()=>{vi.clearAllMocks();fetchOfflineNodes.mockResolvedValue([node,{node_name:"manual",node_uuid:"manual",status:"offline",can_resume:false}]);});
test("offline controls preserve node identity and use the saved resume action", async()=>{
  let finish;updateOfflineNode.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  render(<OfflineNodesPanel/>);await screen.findByText("old");
  const row=screen.getByText("old").closest("tr");
  expect(within(row).getByText(/Example #3/)).toBeVisible();
  fireEvent.click(within(row).getByRole("button",{name:"Resume",exact:true}));
  expect(updateOfflineNode).toHaveBeenCalledWith(node,"resume");
  expect(within(row).getByRole("button",{name:"Retire"})).toBeDisabled();
  await act(async()=>finish({request_id:4}));
  expect(screen.getByText("Queued resume for old.")).toBeVisible();
  expect(within(screen.getByText("manual").closest("tr")).getByRole("button",{name:"Resume",exact:true})).toBeDisabled();
});
test("assignment release failures keep the row usable and explain the error", async()=>{
  updateOfflineNode.mockRejectedValue(new Error("Node became live. Refresh the node list."));
  render(<OfflineNodesPanel/>);await screen.findByText("old");
  fireEvent.click(within(screen.getByText("old").closest("tr")).getByRole("button",{name:"Release assignment"}));
  await waitFor(()=>expect(screen.getByRole("alert")).toHaveTextContent("Node became live"));
  expect(updateOfflineNode).toHaveBeenCalledWith(node,"release-offline");
});

test("unassigned and already retired workers can be released and retired again", async () => {
  fetchOfflineNodes.mockResolvedValue([{ ...node, status: "retired", pool_run_id: null }]);
  updateOfflineNode.mockResolvedValue({ rows_updated: 1 });
  render(<OfflineNodesPanel />);
  await screen.findByText("old");
  expect(screen.getByRole("button", { name: "Release assignment" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Retire" })).toBeEnabled();
});
