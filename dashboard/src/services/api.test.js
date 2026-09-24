import { afterEach, expect, test, vi } from "vitest";
import { deleteRun, fetchSession, fetchRunPerformance, fetchRunPerformanceGraphs } from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

const removalResponse = (status, error = null) => ({
  ok: true,
  json: async () => ({ operation_id: "operation-1", run_id: 1, status, error, poll_after_ms: 1000 }),
});

test("deletion polls short requests and reports success only after the background operation completes", async () => {
  vi.useFakeTimers();
  const fetch = vi.fn()
    .mockResolvedValueOnce(removalResponse("running"))
    .mockResolvedValueOnce(removalResponse("running"))
    .mockResolvedValueOnce(removalResponse("completed"));
  vi.stubGlobal("fetch", fetch);
  let completed = false;
  const pending = deleteRun(1).then((result) => { completed = true; return result; });
  await vi.advanceTimersByTimeAsync(1000);
  expect(completed).toBe(false);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch.mock.calls[0][0]).toBe("/api/runs/1");
  expect(fetch.mock.calls[0][1].method).toBe("DELETE");
  expect(fetch.mock.calls[1][0]).toBe("/api/run-removals/operation-1");
  await vi.advanceTimersByTimeAsync(1000);
  await expect(pending).resolves.toMatchObject({ status: "completed" });
});

test("deletion propagates a background failure instead of claiming success on acceptance", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("fetch", vi.fn()
    .mockResolvedValueOnce(removalResponse("running"))
    .mockResolvedValueOnce(removalResponse("failed", "workers are still draining; run retained")));
  const pending = expect(deleteRun(1)).rejects.toThrow("workers are still draining; run retained");
  await vi.advanceTimersByTimeAsync(1000);
  await pending;
});

test("canceling status polling stops further requests", async () => {
  vi.useFakeTimers();
  const fetch = vi.fn().mockResolvedValue(removalResponse("running"));
  vi.stubGlobal("fetch", fetch);
  const controller = new AbortController();
  const pending = expect(deleteRun(1, controller.signal)).rejects.toMatchObject({ name: "AbortError" });
  await vi.advanceTimersByTimeAsync(0);
  controller.abort();
  await pending;
  await vi.advanceTimersByTimeAsync(5000);
  expect(fetch).toHaveBeenCalledTimes(1);
});

test("gateway timeouts explain that the operation may still be running", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
    ok: false, status: 504,
    headers: { get: () => "text/html" },
    text: async () => "<html>504 Gateway Time-out</html>",
  }));
  await expect(deleteRun(1)).rejects.toThrow("gateway timeout (504): the request took too long and may still be running");
});

test("session startup uses POST so same-origin browsers send their Origin", async () => {
  const fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({ authenticated: true }),
  });
  vi.stubGlobal("fetch", fetch);
  expect(await fetchSession()).toEqual({ authenticated: true });
  expect(fetch).toHaveBeenCalledWith("/api/auth/session", expect.objectContaining({
    method: "POST",
    credentials: "include",
  }));
});


test("summary and graph APIs encode identical rolling, all and custom ranges", async () => {
  const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
  vi.stubGlobal("fetch", fetch);
  for (const [selection, query] of [
    [{ seconds: 30 }, "window_seconds=30"],
    [{ seconds: 300 }, "window_seconds=300"],
    [null, ""],
    [{ start: 1000, end: 1040, follow: false }, "start_ms=1000&end_ms=1040"],
    [{ start: 1000, end: 1040, follow: true }, "window_seconds=0.04"],
  ]) {
    await fetchRunPerformance(1, selection);
    expect(fetch.mock.lastCall[0]).toBe(`/api/runs/1/performance${query ? `?${query}` : ""}`);
    await fetchRunPerformanceGraphs(1, selection);
    expect(fetch.mock.lastCall[0]).toBe(`/api/runs/1/performance/graphs${query ? `?${query}` : ""}`);
  }
});
