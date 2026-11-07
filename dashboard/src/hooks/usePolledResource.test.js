import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { usePolledResource } from "./usePolledResource";

test("loading is distinct from an empty response and old requests cannot replace a new resource", async () => {
  let resolveOld;
  let resolveNew;
  const oldSource = vi.fn(() => new Promise((resolve) => { resolveOld = resolve; }));
  const newSource = vi.fn(() => new Promise((resolve) => { resolveNew = resolve; }));
  const { result, rerender } = renderHook(({ fetchResource, enabled = true }) =>
    usePolledResource({ initialData: [], fetchResource, pollMs: 60000, enabled }),
  { initialProps: { fetchResource: oldSource } });
  expect(result.current.isLoading).toBe(true);
  expect(result.current.isFetching).toBe(true);
  rerender({ fetchResource: newSource });
  await act(async () => resolveOld(["stale"]));
  expect(result.current.data).toEqual([]);
  expect(result.current.isFetching).toBe(true);
  await act(async () => resolveNew([]));
  await waitFor(() => expect(result.current.isLoading).toBe(false));
  expect(result.current.isFetching).toBe(false);
  expect(result.current.data).toEqual([]);
  rerender({ fetchResource: newSource, enabled: false });
  expect(result.current.isLoading).toBe(false);
});

test("failures end initial loading and expose the error", async () => {
  const fetchResource = vi.fn().mockRejectedValue(new Error("offline"));
  const { result } = renderHook(() => usePolledResource({ initialData: [], fetchResource, pollMs: 60000 }));
  await waitFor(() => expect(result.current.error?.message).toBe("offline"));
  expect(result.current.isLoading).toBe(false);
  expect(result.current.isFetching).toBe(false);
});
