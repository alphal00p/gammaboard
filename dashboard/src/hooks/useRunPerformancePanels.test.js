import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { fetchRunPerformance } from "../services/api";
import { useRunPerformancePanels } from "./useRunPerformancePanels";

vi.mock("../services/api", () => ({ fetchRunPerformance: vi.fn() }));
const response = (id, value) => ({
  source_id: `performance:${id}`, panels: [{ panel_id: "busy_rates", kind: "table" }],
  updates: [{ mode: "replace", panel: { panel_id: "busy_rates", kind: "table", rows: [[value]] } }],
  reset_required: true, poll_after_ms: null,
});

test("retains measurements during a window refresh, ignores superseded requests and resets for another run", async () => {
  const requests = [];
  fetchRunPerformance.mockImplementation((run, selection, node, signal) => new Promise((resolve, reject) => {
    requests.push({ run, selection, node, signal, resolve, reject });
  }));
  const { result, rerender } = renderHook((props) => useRunPerformancePanels(props), {
    initialProps: { runId: 1, selection: { seconds: 30 } },
  });
  await act(async () => requests[0].resolve(response(1, 10)));
  const original = result.current.panelStates;
  rerender({ runId: 1, selection: { seconds: 300 } });
  expect(requests[1].selection).toEqual({ seconds: 300 });
  expect(result.current.refreshing).toBe(true);
  expect(result.current.panelStates).toBe(original);
  rerender({ runId: 1, selection: null });
  expect(requests[1].signal.aborted).toBe(true);
  await act(async () => requests[1].reject(new Error("superseded transport failure")));
  expect(result.current.error).toBeNull();
  expect(result.current.panelStates).toBe(original);
  // Finishing an aborted poll must not lose the next poll's abort controller.
  rerender({ runId: 1, selection: { seconds: 60 } });
  expect(requests[2].signal.aborted).toBe(true);
  await act(async () => requests[2].resolve(response(1, 30)));
  expect(result.current.panelStates).toBe(original);
  await act(async () => requests[3].resolve(response(1, 40)));
  expect(result.current.panelStates[0].rows).toEqual([[40]]);
  expect(result.current.refreshing).toBe(false);
  rerender({ runId: 2, selection: null });
  await waitFor(() => expect(result.current.panelStates).toEqual([]));
  expect(requests.at(-1).run).toBe(2);
  await act(async () => requests.at(-1).resolve(response(2, 50)));
  expect(result.current.panelStates[0].rows).toEqual([[50]]);
});
