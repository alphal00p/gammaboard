import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { useCampaign } from "./useCampaign";
import { fetchCampaign } from "../services/api";
import { clearResourceCache } from "../services/resourceCache";
vi.mock("../services/api",()=>({fetchCampaign:vi.fn()}));
afterEach(()=>{vi.useRealTimers();vi.restoreAllMocks();vi.clearAllMocks();clearResourceCache();});

test("campaign views refresh every ten seconds, pause when hidden and reuse cached panels",async()=>{
  vi.useFakeTimers();
  let visible="visible";
  vi.spyOn(document,"visibilityState","get").mockImplementation(()=>visible);
  let pending;
  const signals=[];
  fetchCampaign.mockImplementation((run,task,options,signal)=>{signals.push(signal);return Promise.resolve({panels:[],version:signals.length});});
  const {result,rerender,unmount}=renderHook(({paused=false})=>useCampaign({runId:1,taskId:"2",section:"diagnostics",paused}),{initialProps:{}});
  await act(async()=>{});
  expect(result.current.data.version).toBe(1);
  await act(async()=>vi.advanceTimersByTimeAsync(9999));expect(fetchCampaign).toHaveBeenCalledTimes(1);
  await act(async()=>vi.advanceTimersByTimeAsync(1));expect(fetchCampaign).toHaveBeenCalledTimes(2);
  fetchCampaign.mockImplementation((run,task,options,signal)=>{signals.push(signal);return new Promise(resolve=>{pending=resolve;});});
  await act(async()=>vi.advanceTimersByTimeAsync(10000));expect(fetchCampaign).toHaveBeenCalledTimes(3);
  await act(async()=>vi.advanceTimersByTimeAsync(40000));expect(fetchCampaign).toHaveBeenCalledTimes(3);
  rerender({paused:true});expect(signals.at(-1).aborted).toBe(true);
  await act(async()=>pending({version:99}));expect(result.current.data.version).toBe(2);
  rerender({paused:false});expect(fetchCampaign).toHaveBeenCalledTimes(4);
  act(()=>{visible="hidden";document.dispatchEvent(new Event("visibilitychange"));});
  expect(signals.at(-1).aborted).toBe(true);
  await act(async()=>vi.advanceTimersByTimeAsync(30000));expect(fetchCampaign).toHaveBeenCalledTimes(4);
  act(()=>{visible="visible";document.dispatchEvent(new Event("visibilitychange"));});expect(fetchCampaign).toHaveBeenCalledTimes(5);
  unmount();
  const revisit=renderHook(()=>useCampaign({runId:1,taskId:"2",section:"diagnostics"}));
  expect(revisit.result.current.data.version).toBe(2);expect(revisit.result.current.isLoading).toBe(false);expect(revisit.result.current.isFetching).toBe(true);
  await act(async()=>pending({version:6}));expect(revisit.result.current.data.version).toBe(6);
  revisit.unmount();
});

test("failed refresh retains cached data and different sections never share it",async()=>{
  fetchCampaign.mockResolvedValueOnce({version:1}).mockRejectedValue(new Error("timeout"));
  const {result,rerender}=renderHook(({section})=>useCampaign({runId:1,taskId:"2",section}),{initialProps:{section:"overview"}});
  await act(async()=>{});
  await act(async()=>result.current.refresh());
  expect(result.current.data.version).toBe(1);expect(result.current.error.message).toBe("timeout");
  rerender({section:"diagnostics"});await act(async()=>{});
  expect(result.current.data).toBeNull();
});
