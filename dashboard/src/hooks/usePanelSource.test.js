import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
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
  test("refetches after applying stored panel values for a new source", async () => {
    window.localStorage.setItem(
      "gammaboard.panel-values.source",
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
});
