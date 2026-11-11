import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import WorkspaceTabs, { TabLoadingScope, WorkspacePanel } from "./WorkspaceTabs";
import LoadingStatus from "./LoadingStatus";
import { usePanelSource } from "../../hooks/usePanelSource";

afterEach(() => vi.useRealTimers());

test("descendant loading stays in the fixed tab slot and stops when its workspace is hidden", () => {
  vi.useFakeTimers();
  const tree = (loading, active = true) => <TabLoadingScope>
    <WorkspaceTabs id="test" label="Test views" value="one" onChange={() => {}} tabs={[["one", "One"]]} />
    <WorkspacePanel id="test" value="one" active={active}>
      <LoadingStatus loading={loading} label="Refreshing results…" />
      <div data-testid="content">Retained results</div>
    </WorkspacePanel>
  </TabLoadingScope>;
  const { rerender } = render(tree(false));
  const tabs = screen.getByRole("tablist");
  const content = screen.getByTestId("content");
  rerender(tree(true));
  act(() => vi.advanceTimersByTime(200));
  expect(screen.getByRole("status")).toHaveAccessibleName("Refreshing results…");
  expect(screen.getByRole("status").closest('[data-loading-slot="test"]')).not.toBeNull();
  expect(screen.getByRole("tablist")).toBe(tabs);
  expect(screen.getByTestId("content")).toBe(content);
  rerender(tree(true, false));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

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
