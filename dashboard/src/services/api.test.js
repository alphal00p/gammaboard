import { afterEach, expect, test, vi } from "vitest";
import { fetchSession, fetchRunPerformance, fetchRunPerformanceGraphs } from "./api";

afterEach(() => vi.unstubAllGlobals());

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
