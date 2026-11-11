import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { useTaskOutput } from "./useTaskOutput";
import { fetchRunTaskPanels } from "../services/api";
vi.mock("../services/api", () => ({ fetchRunTaskPanels: vi.fn() }));
afterEach(() => vi.resetAllMocks());
const response = (task, section, text) => ({ source_id: `${task}:${section}`, poll_after_ms: null,
  panels: [{ panel_id: "details", label: "Details", kind: "text", history: "none" }],
  updates: [{ mode: "replace", panel: { panel_id: "details", kind: "text", text } }], available_sections: ["result","observables"] });

test("tabs and tasks reuse their own cached output immediately and revalidate completed tasks", async () => {
  fetchRunTaskPanels.mockImplementation(async (_,task,{section}) => response(task,section,`${task}:${section}`));
  const { result, rerender, unmount } = renderHook((props) => useTaskOutput({ runId: 1, refreshKey: "completed", ...props }),
    { initialProps: { taskId: "1", section: "result" } });
  await waitFor(() => expect(result.current.panelStates[0]?.text).toBe("1:result"));
  rerender({ taskId: "1", section: "observables" });
  expect(result.current.panelStates).toEqual([]);
  await waitFor(() => expect(result.current.panelStates[0]?.text).toBe("1:observables"));
  let finish;
  fetchRunTaskPanels.mockImplementationOnce(() => new Promise((resolve) => { finish=resolve; }));
  rerender({ taskId: "1", section: "result" });
  expect(result.current.panelStates[0].text).toBe("1:result");
  expect(result.current.isLoading).toBe(false);
  expect(result.current.isRevalidating).toBe(true);
  await act(async () => finish(response("1","result","fresh result")));
  expect(result.current.panelStates[0].text).toBe("fresh result");
  expect(result.current.isRevalidating).toBe(false);
  rerender({ taskId: "2", section: "result" });
  expect(result.current.panelStates).toEqual([]);
  await waitFor(() => expect(result.current.panelStates[0]?.text).toBe("2:result"));
  unmount();
  fetchRunTaskPanels.mockImplementation(() => new Promise(()=>{}));
  const again = renderHook(() => useTaskOutput({ runId: 1, taskId: "1", section: "result" }));
  expect(again.result.current.panelStates[0].text).toBe("fresh result");
  expect(again.result.current.isRevalidating).toBe(true);
});

test("current results render before history, and rollback replaces cached history", async () => {
  let finishHistory;
  const spec = { panel_id: "mean_history", kind: "scalar_timeseries", history: "append" };
  const plot = (x) => ({ panel_id: spec.panel_id, kind: spec.kind, points: [{ x, y: x }] });
  fetchRunTaskPanels.mockImplementation(async (_,task,{phase}) => phase === "history" ? new Promise(resolve=>{finishHistory=resolve;}) :
    { source_id: task, poll_after_ms: null, panels: [spec], updates: [{ mode: "replace", panel: plot(30) }] });
  const { result, rerender } = renderHook(({paused}) => useTaskOutput({ runId: 1, taskId: "1", paused }), { initialProps: {paused:false} });
  await waitFor(() => expect(result.current.isHistoryLoading).toBe(true));
  expect(result.current.panelStates[0].points.map(p=>p.x)).toEqual([30]);
  await act(async () => finishHistory({ source_id: "1", cursor: "3:0", poll_after_ms: null, panels:[spec], reset_required:true,
    updates:[{mode:"replace",panel:{...plot(10),points:[{x:10,y:10},{x:20,y:20}]}}] }));
  expect(result.current.panelStates[0].points.map(p=>p.x)).toEqual([10,20,30]);
  rerender({paused:true});
  fetchRunTaskPanels.mockImplementation(async (_,task,{phase}) => ({source_id: task,cursor:"1:0",poll_after_ms:null,panels:[spec],reset_required:phase==="history",
    updates:[{mode:"replace",panel:plot(10)}]}));
  rerender({paused:false});
  await waitFor(() => expect(result.current.panelStates[0].points.map(p=>p.x)).toEqual([10]));
});

test("failed refresh keeps cached data, but authorization failures clear it", async () => {
  fetchRunTaskPanels.mockResolvedValue(response("1","result","saved"));
  const { result, rerender } = renderHook(({paused}) => useTaskOutput({runId:1,taskId:"1",paused}), {initialProps:{paused:false}});
  await waitFor(()=>expect(result.current.panelStates[0]?.text).toBe("saved"));
  rerender({paused:true});
  fetchRunTaskPanels.mockRejectedValue(new Error("Temporary failure"));
  rerender({paused:false});
  await waitFor(()=>expect(result.current.error).toBe("Temporary failure"));
  expect(result.current.panelStates[0].text).toBe("saved");
  rerender({paused:true});
  fetchRunTaskPanels.mockRejectedValue(Object.assign(new Error("Access denied"),{status:403}));
  rerender({paused:false});
  await waitFor(()=>expect(result.current.error).toBe("Access denied"));
  expect(result.current.panelStates).toEqual([]);
});

test("late responses cannot overwrite another task or warm its cache", async () => {
  let finish;
  fetchRunTaskPanels.mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}))
    .mockResolvedValue(response("2","result","second"));
  const {result,rerender}=renderHook(({taskId})=>useTaskOutput({runId:1,taskId}),{initialProps:{taskId:"1"}});
  rerender({taskId:"2"});
  await waitFor(()=>expect(result.current.panelStates[0]?.text).toBe("second"));
  await act(async()=>finish(response("1","result","late first")));
  expect(result.current.panelStates[0].text).toBe("second");
  fetchRunTaskPanels.mockImplementation(()=>new Promise(()=>{}));
  rerender({taskId:"1"});
  expect(result.current.panelStates).toEqual([]);
});
