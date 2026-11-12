import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import CampaignScheduling from "./CampaignScheduling";
import CampaignQueueTuning from "./CampaignQueueTuning";
import { useCampaign } from "../../hooks/useCampaign";
import { updateCampaignScheduling,updateCampaignTuning } from "../../services/api";
vi.mock("../../auth/AuthProvider",()=>({useAuth:()=>({authenticated:true})}));
vi.mock("../../hooks/useCampaign",()=>({useCampaign:vi.fn()}));
vi.mock("../../services/api",()=>({updateCampaignScheduling:vi.fn(),updateCampaignTuning:vi.fn()}));
let resource;
beforeEach(()=>{
 vi.clearAllMocks();
 resource={isFetching:false,error:null,refresh:vi.fn(),data:{state:"active",capacity:1,samplers:1,evaluators:10,reserved_samples:200,
 settings:{allocation:{algorithm:"variance_reduction_rate",max_active_runs:2,min_evaluators_per_run:5,allocation_window_samples:100,min_samples_per_child:20},stop_condition:{min_total_samples:0,max_total_samples:1000,absolute_error:null,relative_error:null}},
 children:[{label:"A",run_id:3,defaults:{max_batch_size:64},tasks:[{id:"30",name:"infer",state:"active",publish_result:true}]},
 {label:"B",run_id:4,defaults:{max_batch_size:128},tasks:[{id:"40",name:"train",state:"completed",publish_result:false}]}]}};
 useCampaign.mockImplementation(()=>resource);
 updateCampaignScheduling.mockResolvedValue({updated:true});updateCampaignTuning.mockResolvedValue({updated:[{}],skipped:[{}]});
});
test("scheduling drafts survive polls and submit their original expected settings",async()=>{
 const original=resource.data.settings;
 const {rerender}=render(<CampaignScheduling runId={1} taskId="2" />);
 expect(screen.getByText(/Sampler availability/)).toBeVisible();
 fireEvent.change(screen.getByRole("spinbutton",{name:"Maximum concurrent children"}),{target:{value:"3"}});
 resource={...resource,data:{...resource.data,settings:{...original,allocation:{...original.allocation,max_active_runs:4}}}};
 rerender(<CampaignScheduling runId={1} taskId="2" />);
 expect(screen.getByRole("spinbutton",{name:"Maximum concurrent children"})).toHaveValue(3);
 fireEvent.click(screen.getByRole("button",{name:"Apply scheduling"}));
 await waitFor(()=>expect(updateCampaignScheduling).toHaveBeenCalled());
 expect(updateCampaignScheduling.mock.calls[0][2]).toMatchObject({expected:original,allocation:{max_active_runs:3}});
});
test("queue tuning defaults to every child and applies only the edited fields",async()=>{
 render(<CampaignQueueTuning runId={1} taskId="2" resource={resource} />);
 expect(screen.getByText(/2 children selected · 1 pending\/active tasks · 1 finished tasks skipped/)).toBeVisible();
 fireEvent.change(screen.getByRole("spinbutton",{name:"Target evaluation time (ms)"}),{target:{value:"500"}});
 fireEvent.click(screen.getByRole("button",{name:"Apply to selected children"}));
 await waitFor(()=>expect(updateCampaignTuning).toHaveBeenCalledWith(1,"2",{children:["A","B"],scope:"all",patch:{target_batch_eval_ms:500}}));
 expect(await screen.findByText(/Updated 1 task/)).toBeVisible();
});
test("finished campaigns cannot be edited",()=>{
 resource.data.state="completed";
 render(<CampaignScheduling runId={1} taskId="2" />);
 expect(screen.getByRole("button",{name:"Apply scheduling"})).toBeDisabled();
 expect(screen.getByText("Completed or failed campaigns are read-only.")).toBeVisible();
});
