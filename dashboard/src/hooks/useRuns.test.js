import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { useRuns } from "./useRuns";
import { fetchRuns } from "../services/api";

vi.mock("../services/api", () => ({ fetchRuns: vi.fn() }));
afterEach(() => vi.resetAllMocks());
const root = { run_id: 1, run_name: "Root" };
const older = { run_id: 2, run_name: "Older root" };
const child = { run_id: 3, run_name: "Child", parent_run_id: 1 };

test("child visibility preserves roots and their loaded pages while the child catalog refreshes", async () => {
  let finishChildren;
  fetchRuns.mockImplementation(({ includeChildren, offset }) => includeChildren
    ? new Promise((resolve) => { finishChildren = resolve; })
    : Promise.resolve(offset ? { items: [older], nextOffset: null } : { items: [root], nextOffset: 100 }));
  const { result, rerender } = renderHook(({ includeChildren }) => useRuns({ includeChildren, refreshInterval: 60000 }),
    { initialProps: { includeChildren: false } });
  await waitFor(() => expect(result.current.runs).toEqual([root]));
  await act(async () => result.current.loadMoreRuns());
  expect(result.current.runs).toEqual([root, older]);
  const rootRequests = fetchRuns.mock.calls.filter(([args]) => !args.includeChildren).length;
  rerender({ includeChildren: true });
  expect(result.current.runs).toEqual([root, older]);
  expect(result.current.isLoadingRuns).toBe(true);
  await act(async () => finishChildren({ items: [root, child], nextOffset: null }));
  expect(result.current.runs).toEqual([root, older, child]);
  rerender({ includeChildren: false });
  expect(result.current.runs).toEqual([root, older]);
  rerender({ includeChildren: true });
  expect(result.current.runs).toEqual([root, older, child]);
  expect(result.current.isLoadingRuns).toBe(false);
  expect(fetchRuns.mock.calls.filter(([args]) => !args.includeChildren)).toHaveLength(rootRequests);
});

test("hiding children aborts pagination without allowing its late result to populate the catalog", async () => {
  let finishPage;
  let pageSignal;
  fetchRuns.mockImplementation(({ includeChildren, offset }, signal) => {
    if (!includeChildren) return Promise.resolve({ items: [root], nextOffset: null });
    if (!offset) return Promise.resolve({ items: [child], nextOffset: 100 });
    pageSignal = signal;
    return new Promise((resolve) => { finishPage = resolve; });
  });
  const { result, rerender } = renderHook(({ includeChildren }) => useRuns({ includeChildren, refreshInterval: 60000 }),
    { initialProps: { includeChildren: true } });
  await waitFor(() => expect(result.current.runs).toEqual([root, child]));
  let pending;
  act(() => { pending = result.current.loadMoreRuns(); });
  rerender({ includeChildren: false });
  expect(pageSignal.aborted).toBe(true);
  await act(async () => { finishPage({ items: [{ ...child, run_id: 4 }], nextOffset: null }); await pending; });
  rerender({ includeChildren: true });
  expect(result.current.runs).toEqual([root, child]);
});
