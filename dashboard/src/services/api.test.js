import { afterEach, expect, test, vi } from "vitest";
import { deleteRun, fetchSession, fetchRunPerformance, fetchRunPerformanceGraphs, fetchCampaign, updateOfflineNode, editRunTask, createRun, saveTemplateFile } from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

test("restoration uses a distinct action from individual start and fences the worker identity", async () => {
  const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ request_id: "7" }) });
  vi.stubGlobal("fetch", fetch);
  const node = { node_name: "worker / one", node_uuid: "previous-process" };
  for (const action of ["start", "restore", "cancel-restoration", "unassign-offline"]) {
    await updateOfflineNode(node, action);
    const payload = JSON.parse(fetch.mock.lastCall[1].body);
    expect(fetch.mock.lastCall[0]).toBe("/api/v1/manage");
    const identity = action === "start" || action === "restore" ? payload.source : payload;
    expect(identity).toMatchObject({ name: node.node_name, expected_uuid: node.node_uuid });
    if (action === "start" || action === "restore") {
      expect(payload.operation).toBe("worker_launch");
      expect(identity.restore_only).toBe(action === "restore");
    } else expect(payload.operation).toBe(action === "unassign-offline" ? "worker_unassign" : "worker_cancel_restoration");
  }
});

const removalResponse = (status, error = null, accepted = false) => ({
  ok: true,
  json: async () => { const operation = { operation_id: "operation-1", run_id: 1, status, error, poll_after_ms: 1000 }; return accepted ? { operations: [operation] } : operation; },
});

test("deletion polls short requests and reports success only after the background operation completes", async () => {
  vi.useFakeTimers();
  const fetch = vi.fn()
    .mockResolvedValueOnce(removalResponse("running", null, true))
    .mockResolvedValueOnce(removalResponse("running"))
    .mockResolvedValueOnce(removalResponse("completed"));
  vi.stubGlobal("fetch", fetch);
  let completed = false;
  const pending = deleteRun(1).then((result) => { completed = true; return result; });
  await vi.advanceTimersByTimeAsync(1000);
  expect(completed).toBe(false);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch.mock.calls[0][0]).toBe("/api/v1/manage");
  expect(fetch.mock.calls[0][1].method).toBe("POST");
  expect(JSON.parse(fetch.mock.calls[1][1].body)).toEqual({ operation: "run_removal_status", operation_id: "operation-1" });
  await vi.advanceTimersByTimeAsync(1000);
  await expect(pending).resolves.toMatchObject({ status: "completed" });
});

test("deletion propagates a background failure instead of claiming success on acceptance", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("fetch", vi.fn()
    .mockResolvedValueOnce(removalResponse("running", null, true))
    .mockResolvedValueOnce(removalResponse("failed", "workers are still draining; run retained")));
  const pending = expect(deleteRun(1)).rejects.toThrow("workers are still draining; run retained");
  await vi.advanceTimersByTimeAsync(1000);
  await pending;
});

test("canceling status polling stops further requests", async () => {
  vi.useFakeTimers();
  const fetch = vi.fn().mockResolvedValue(removalResponse("running", null, true));
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


test("summary, graph and campaign APIs encode identical rolling, all and custom ranges", async () => {
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
    await fetchCampaign(1, "2", { section: "overview", selection });
    expect(fetch.mock.lastCall[0]).toBe(`/api/runs/1/tasks/2/campaign?section=overview${query ? `&${query}` : ""}`);
  }
});


test("dashboard mutations use the canonical contract and preserve bigint task IDs", async () => {
  const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
  vi.stubGlobal("fetch", fetch);
  await editRunTask(1, "9007199254740993", "new", "previous");
  expect(JSON.parse(fetch.mock.lastCall[1].body)).toEqual({ operation: "task", run: "1",
    action: { action: "edit", id: "9007199254740993", toml: "new", expected_toml: "previous" } });
  await saveTemplateFile("nodes", { name: "fleet", toml: "[[groups]]" });
  expect(JSON.parse(fetch.mock.lastCall[1].body)).toMatchObject({ operation: "template", kind: "worker" });
  fetch.mockResolvedValueOnce({ ok: true, json: async () => ({ paths: { run_templates_dir: "/shared/templates/runs" } }) })
    .mockResolvedValueOnce({ ok: true, json: async () => ({ created: { run_id: 12 } }) });
  expect(await createRun("name='new'")).toEqual({ run_id: 12 });
  expect(JSON.parse(fetch.mock.lastCall[1].body)).toMatchObject({ operation: "run_create", base_dir: "/shared/templates/runs", start: false });
});
