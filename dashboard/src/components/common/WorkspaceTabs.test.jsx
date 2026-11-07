import { act, render } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { WorkspacePanel } from "./WorkspaceTabs";
import { usePanelSource } from "../../hooks/usePanelSource";

afterEach(() => vi.useRealTimers());

test("inactive workspaces abort polling and retain cursors, history and preferences", async () => {
  vi.useFakeTimers();
  const initial = { source_id: "task:7", cursor: "8:0", panels: [{ panel_id: "mean", kind: "scalar_timeseries" }],
    updates: [{ mode: "replace", panel: { panel_id: "mean", kind: "scalar_timeseries", points: [{ x: 1, y: 2 }] } }] };
  let late;
  const fetchPanels = vi.fn().mockResolvedValueOnce(initial)
    .mockImplementationOnce(() => new Promise((resolve) => { late = resolve; }))
    .mockResolvedValue({ source_id: "task:7", cursor: "9:0", panels: [], updates: [
      { mode: "append", panel: { panel_id: "mean", kind: "scalar_timeseries", points: [{ x: 2, y: 3 }] } },
    ] });
  let source;
  function Reader() { source = usePanelSource({ pollMs: 1000, fetchPanels }); return null; }
  const tree = (active) => <WorkspacePanel id="test" value="source" active={active}><Reader /></WorkspacePanel>;
  const { rerender } = render(tree(true));
  await act(async () => {});
  act(() => source.setPanelValue("mean", { zoom: { start: 10, end: 90 } }, false));
  await act(async () => vi.advanceTimersByTime(1000));
  expect(fetchPanels).toHaveBeenCalledTimes(2);
  rerender(tree(false));
  expect(fetchPanels.mock.calls[1][1].aborted).toBe(true);
  await act(async () => { vi.advanceTimersByTime(5000); late({ ...initial, cursor: "stale" }); });
  expect(fetchPanels).toHaveBeenCalledTimes(2);
  expect(source.panelStates[0].points).toEqual([{ x: 1, y: 2 }]);
  rerender(tree(true));
  await act(async () => {});
  expect(fetchPanels.mock.calls[2][0]).toMatchObject({ cursor: "8:0", panelState: { mean: { zoom: { start: 10, end: 90 } } } });
  expect(source.panelSpecs).toHaveLength(1);
  expect(source.panelStates[0].points).toEqual([{ x: 1, y: 2 }, { x: 2, y: 3 }]);
});
