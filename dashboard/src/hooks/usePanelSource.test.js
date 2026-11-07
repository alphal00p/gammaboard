import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { usePanelSource } from "./usePanelSource";

const selectPanelResponse = {
  source_id: "source",
  panels: [
    {
      panel_id: "mode",
      kind: "select",
      label: "Mode",
      history: "none",
      state: {
        kind: "select",
        default_value: "relative",
        options: [
          { value: "relative", label: "Relative" },
          { value: "log", label: "Log" },
        ],
      },
    },
  ],
  updates: [],
  poll_after_ms: null,
};

describe("usePanelSource", () => {
  beforeEach(() => localStorage.clear());
  test("shows fetching without clearing existing panels and ignores an aborted refresh", async () => {
    let finish;
    const fetchPanels = vi.fn().mockResolvedValueOnce(selectPanelResponse)
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result, rerender } = renderHook(({ fetchPanels, refreshKey }) => usePanelSource({ fetchPanels, refreshKey }),
      { initialProps: { fetchPanels, refreshKey: 0 } });
    await waitFor(() => expect(result.current.panelSpecs).toHaveLength(1));
    expect(result.current.isLoading).toBe(false);
    rerender({ fetchPanels, refreshKey: 1 });
    await waitFor(() => expect(result.current.isFetching).toBe(true));
    expect(result.current.panelSpecs).toHaveLength(1);
    const next = vi.fn().mockResolvedValue({ source_id: "new", panels: [], updates: [], poll_after_ms: null });
    rerender({ fetchPanels: next, refreshKey: 1 });
    await waitFor(() => expect(result.current.sourceId).toBe("new"));
    await act(async () => finish(selectPanelResponse));
    expect(result.current.sourceId).toBe("new");
    expect(result.current.panelSpecs).toEqual([]);
    expect(result.current.isLoading).toBe(false);
    expect(result.current.isFetching).toBe(false);
  });
  test("a task state change refreshes a source whose polling had stopped", async () => {
    const fetchPanels = vi.fn(async () => selectPanelResponse);
    const { rerender } = renderHook(({ refreshKey }) => usePanelSource({ fetchPanels, refreshKey }),
      { initialProps: { refreshKey: "pending" } });
    await waitFor(() => expect(fetchPanels).toHaveBeenCalledOnce());
    rerender({ refreshKey: "active" });
    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(2));
  });
  test("refetches after applying stored panel values for a new source", async () => {
    window.localStorage.setItem(
      "gammaboard.panel-values.v1.source",
      JSON.stringify({
        mode: "log",
      }),
    );
    const fetchPanels = vi.fn(async () => selectPanelResponse);

    renderHook(() =>
      usePanelSource({
        enabled: true,
        pollMs: 60_000,
        fetchPanels,
        useCursor: true,
      }),
    );

    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(2));
    expect(fetchPanels.mock.calls[0][0].panelState).toEqual({});
    expect(fetchPanels.mock.calls[1][0].panelState).toMatchObject({ mode: "log" });
  });

  test("sends changed panel values with immediate triggered poll", async () => {
    const fetchPanels = vi.fn(async () => selectPanelResponse);

    const { result } = renderHook(() =>
      usePanelSource({
        enabled: true,
        pollMs: 60_000,
        fetchPanels,
        useCursor: true,
      }),
    );

    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(1));

    act(() => {
      result.current.setPanelValue("mode", "log");
    });

    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(2));
    expect(fetchPanels.mock.calls[1][0].panelState).toMatchObject({ mode: "log" });
  });

  test("clears panels when a complete response becomes empty", async () => {
    const fetchPanels = vi
      .fn()
      .mockResolvedValueOnce({
        source_id: "source",
        panels: [{ panel_id: "summary", kind: "key_value", label: "Summary", history: "none" }],
        updates: [],
        poll_after_ms: 1,
      })
      .mockResolvedValue({
        source_id: "source",
        panels: [],
        updates: [],
        reset_required: true,
        poll_after_ms: null,
      });

    const { result } = renderHook(() =>
      usePanelSource({
        enabled: true,
        pollMs: 60_000,
        fetchPanels,
        useCursor: false,
      }),
    );

    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(result.current.panelSpecs).toEqual([]));
  });

  test("retains panel specs and state when an incremental cursor response has no updates", async () => {
    const panelSpec = {
      panel_id: "controller_children",
      kind: "table",
      label: "Sub-runs",
      history: "none",
    };
    const childTable = {
      panel_id: "controller_children",
      kind: "table",
      columns: ["run", "status"],
      rows: [[42, "completed"]],
    };
    const fetchPanels = vi
      .fn()
      .mockResolvedValueOnce({
        source_id: "task:1",
        panels: [panelSpec],
        updates: [{ mode: "replace", panel: childTable }],
        cursor: "8:0",
        poll_after_ms: 1,
      })
      .mockResolvedValue({
        source_id: "task:1",
        panels: [],
        updates: [],
        cursor: "8:0",
        poll_after_ms: null,
      });

    const { result } = renderHook(() =>
      usePanelSource({
        enabled: true,
        pollMs: 60_000,
        fetchPanels,
        useCursor: true,
      }),
    );

    await waitFor(() => expect(fetchPanels).toHaveBeenCalledTimes(2));
    expect(result.current.panelSpecs).toEqual([panelSpec]);
    expect(result.current.panelStates).toEqual([childTable]);
  });
});
