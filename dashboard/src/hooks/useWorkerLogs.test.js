import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { fetchRuntimeLogPage } from "../services/api";
import { useWorkerLogs } from "./useWorkerLogs";

vi.mock("../services/api", () => ({ fetchRuntimeLogPage: vi.fn() }));
beforeEach(() => vi.clearAllMocks());

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
