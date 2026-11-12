import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { fetchRuntimeLogPage } from "../services/api";
import { useWorkerLogs } from "./useWorkerLogs";

vi.mock("../services/api", () => ({ fetchRuntimeLogPage: vi.fn() }));
const activity = vi.hoisted(() => ({ active: true }));
vi.mock("../components/common/WorkspaceTabs", () => ({ useWorkspaceActive: () => activity.active }));
beforeEach(() => { vi.resetAllMocks(); activity.active = true; });

test("returning to Logs keeps older pages until an explicit refresh", async () => {
  fetchRuntimeLogPage.mockImplementation(async ({ beforeId }) => beforeId
    ? { items: [{ id: "2" }], next_before_id: "2", has_more_older: true }
    : { items: [{ id: "3" }], next_before_id: "3", has_more_older: true });
  const { result, rerender } = renderHook(() => useWorkerLogs());
  await waitFor(() => expect(result.current.items).toEqual([{ id: "3" }]));
  expect(fetchRuntimeLogPage).toHaveBeenCalledOnce();
  act(() => result.current.loadOlder());
  await waitFor(() => expect(result.current.items).toHaveLength(2));
  activity.active = false; rerender();
  activity.active = true; rerender();
  expect(result.current.items).toEqual([{ id: "3" }, { id: "2" }]);
  expect(fetchRuntimeLogPage).toHaveBeenCalledTimes(2);

  let finishOlder;
  fetchRuntimeLogPage.mockImplementationOnce(() => new Promise((resolve) => { finishOlder = resolve; }));
  act(() => result.current.loadOlder());
  const signal = fetchRuntimeLogPage.mock.lastCall[1];
  activity.active = false; rerender();
  expect(signal.aborted).toBe(true);
  activity.active = true; rerender();
  expect(result.current.isLoading).toBe(false);
  expect(result.current.items).toHaveLength(2);
  expect(fetchRuntimeLogPage).toHaveBeenCalledTimes(3);
  await act(async () => finishOlder({ items: [{ id: "1" }], has_more_older: false }));
  expect(result.current.items).toHaveLength(2);
  act(() => result.current.refresh());
  await waitFor(() => expect(result.current.items).toEqual([{ id: "3" }]));
  expect(fetchRuntimeLogPage).toHaveBeenCalledTimes(4);
});

test("late pagination cannot append rows after a filter change", async () => {
  let finishOlder;
  let olderSignal;
  fetchRuntimeLogPage.mockImplementation(({ beforeId, level }, signal) => {
    if (beforeId) {
      olderSignal = signal;
      return new Promise((resolve) => { finishOlder = resolve; });
    }
    return Promise.resolve({ items: [{ id: level === "error" ? "5" : "3" }], next_before_id: "3", has_more_older: true });
  });
  const { result } = renderHook(() => useWorkerLogs());
  await waitFor(() => expect(result.current.items).toEqual([{ id: "3" }]));
  act(() => result.current.loadOlder());
  act(() => result.current.setFilters((previous) => ({ ...previous, level: "error" })));
  expect(olderSignal.aborted).toBe(true);
  await waitFor(() => expect(result.current.items).toEqual([{ id: "5" }]));
  await act(async () => finishOlder({ items: [{ id: "2" }], has_more_older: false }));
  expect(result.current.items).toEqual([{ id: "5" }]);
});

test("only indicates initial loading for each filter selection, including empty log results", async () => {
  let finish;
  fetchRuntimeLogPage.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const { result } = renderHook(() => useWorkerLogs());
  await waitFor(() => expect(result.current.isLoading).toBe(true));
  expect(result.current.isInitialLoading).toBe(true);
  await act(async () => finish({ items: [], has_more_older: false }));
  expect(result.current.isInitialLoading).toBe(false);
  act(() => result.current.refresh());
  expect(result.current.isLoading).toBe(true);
  expect(result.current.isInitialLoading).toBe(false);
  await act(async () => finish({ items: [], has_more_older: false }));
  act(() => result.current.setFilters((previous) => ({ ...previous, level: "error" })));
  expect(result.current.isInitialLoading).toBe(true);
  await act(async () => finish({ items: [], has_more_older: false }));
  expect(result.current.isInitialLoading).toBe(false);
});
