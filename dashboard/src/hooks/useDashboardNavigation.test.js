import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import { DEFAULT_NAVIGATION, navigationHash, readNavigation, useDashboardNavigation } from "./useDashboardNavigation";

beforeEach(() => window.history.replaceState(null, "", "/"));

test("links preserve bigint task IDs and reject invalid sections and run IDs", () => {
  const state = { ...DEFAULT_NAVIGATION, runId: 12, taskId: "9007199254740993", children: true, taskTab: "observables" };
  expect(readNavigation(navigationHash(state))).toEqual(state);
  expect(readNavigation("#workspace=missing&run=-3&task=oops&runTab=gone")).toEqual(DEFAULT_NAVIGATION);
});

test("navigation keeps sibling views and restores browser history", () => {
  const { result } = renderHook(useDashboardNavigation);
  act(() => result.current[1]({ runId: 1, runTab: "performance", performanceTab: "graphs" }));
  const runLocation = window.location.hash;
  act(() => result.current[1]({ workspace: "system", systemTab: "database" }));
  expect(result.current[0].performanceTab).toBe("graphs");
  act(() => {
    window.history.replaceState(null, "", `/${runLocation}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  expect(result.current[0]).toMatchObject({ workspace: "runs", runId: 1, runTab: "performance", performanceTab: "graphs" });
  const unchanged = result.current[0];
  act(() => result.current[1]({ workspace: "runs" }));
  expect(result.current[0]).toBe(unchanged);
});
