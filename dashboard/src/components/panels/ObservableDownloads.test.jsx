import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import TablePanel from "./TablePanel";
import { downloadBlob, downloadTextFile } from "./FigureExportActions";
import { renderHistogramBundlePdf } from "./histogramPdf";

vi.mock("./histogramPdf", () => ({ renderHistogramBundlePdf: vi.fn() }));
vi.mock("./FigureExportActions", async (original) => ({ ...await original(), downloadBlob: vi.fn(), downloadTextFile: vi.fn() }));
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

test("campaign JSON/HwU fetch on demand and default to aggregate only", async () => {
  const source = { run_id: 7, task_id: "9007199254740993" };
  const payload = { export_source: source, actions: { export: true }, histograms: {
    energy: { title: "Energy", bins: [{ start: 0, stop: 1, value: 2, error: 0.1 }] },
  } };
  const fetch = vi.fn(async () => ({ ok: true, json: async () => ({ filename: "bundle.json", contents: "{}" }) }));
  vi.stubGlobal("fetch", fetch);
  render(<TablePanel state={{ panel_id: "campaign_histograms", columns: ["Title"], rows: [["Energy"]], payload }} />);
  const checkbox = screen.getByRole("checkbox", { name: "Include child observables" });
  expect(checkbox).not.toBeChecked();
  expect(fetch).not.toHaveBeenCalled();
  for (const include of [false, true]) {
    if (include) fireEvent.click(checkbox);
    for (const format of ["JSON", "HwU"]) {
      fireEvent.click(screen.getByRole("button", { name: format, exact: true }));
      await waitFor(() => expect(screen.getByRole("button", { name: format, exact: true })).toBeEnabled());
      expect(fetch.mock.lastCall[0]).toBe("/api/histogram-bundle/campaign-export");
      expect(JSON.parse(fetch.mock.lastCall[1].body)).toEqual({ ...source, format: format.toLowerCase(), include_children: include });
    }
  }
});

describe.each(["campaign_histograms", "gammaloop_histogram_bundle"])("%s downloads", (panelId) => {
  test("keeps explanation text behind the panel information button while import warnings remain visible", () => {
    const note = panelId === "campaign_histograms" ? "Child estimates are combined in quadrature." : "Pooled means and sample-variance errors.";
    const { rerender } = render(<TablePanel title="Observables" state={{ panel_id: panelId, columns: ["Observable"], rows: [["energy"]],
      payload: { estimator_note: note, description: "Additional estimator details.", actions: { upload_bundle: true, export: true }, histograms: { energy: { bins: [] } } } }}
      bundleUploadError="Import failed" />);
    expect(screen.queryByText(note)).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Import failed");
    fireEvent.click(screen.getByRole("button", { name: "Observables information" }));
    expect(screen.getByRole("dialog", { name: "Observables information" })).toHaveTextContent(note);
    expect(screen.getByRole("dialog")).toHaveTextContent("Additional estimator details.");
    // Empty bundles retain access to the same information and upload controls.
    rerender(<TablePanel title="Observables" state={{ panel_id: panelId, columns: [], rows: [],
      payload: { estimator_note: note, actions: { upload_bundle: true }, histograms: {} } }} />);
    expect(screen.getByText("No observables available.")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toHaveTextContent(note);
  });
  test("downloads the whole bundle and lets PDF exports include comparisons independently", async () => {
    const payload = { actions: { export: true }, histograms: {
      energy: { title: "Energy", bins: [{ start: 0, stop: 1, value: 2, error: 0.1 }] },
      angle: { title: "Angle", bins: [{ start: -1, stop: 1, value: 4, error: 0.2 }] },
    } };
    const state = { panel_id: panelId, columns: ["Observable"], rows: [["energy"], ["angle"]],
      payload, selected_value: { selected_histogram: "energy" } };
    const fetch = vi.fn(async (_url, request) => {
      const { format } = JSON.parse(request.body);
      return { ok: true, json: async () => ({ filename: `histograms.${format}`, contents: `full-${format}`, mime_type: "text/plain" }) };
    });
    vi.stubGlobal("fetch", fetch);
    const { rerender } = render(<TablePanel state={state} />);
    expect(screen.queryByRole("checkbox", { name: "Include comparisons in PDF" })).not.toBeInTheDocument();
    for (const format of ["JSON", "HwU"]) {
      fireEvent.click(screen.getByRole("button", { name: format, exact: true }));
      await waitFor(() => expect(downloadTextFile).toHaveBeenCalledWith(`histograms.${format.toLowerCase()}`, `full-${format.toLowerCase()}`, "text/plain"));
      expect(JSON.parse(fetch.mock.lastCall[1].body).payload.histograms).toEqual(payload.histograms);
    }
    const bundles = [{ id: "reference", label: "reference.json", histograms: payload.histograms }];
    rerender(<TablePanel state={state} uploadedBundles={bundles} />);
    const checkbox = screen.getByRole("checkbox", { name: "Include comparisons in PDF" });
    expect(checkbox).not.toBeChecked();
    let finish;
    renderHistogramBundlePdf.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    fireEvent.click(checkbox);
    fireEvent.click(screen.getByRole("button", { name: "Download plots PDF" }));
    await waitFor(() => expect(renderHistogramBundlePdf).toHaveBeenCalledOnce());
    expect(renderHistogramBundlePdf.mock.lastCall[0]).toMatchObject({ payload, value: state.selected_value,
      uploadedBundles: bundles, includeComparisons: true });
    expect(checkbox).toBeDisabled();
    expect(screen.getByRole("button", { name: "JSON", exact: true })).toBeDisabled();
    const blob = new Blob(["%PDF-1.4"], { type: "application/pdf" });
    await act(async () => finish(blob));
    expect(downloadBlob).toHaveBeenCalledWith(`${panelId}.pdf`, blob);
    fireEvent.click(checkbox);
    renderHistogramBundlePdf.mockResolvedValue(blob);
    fireEvent.click(screen.getByRole("button", { name: "Download plots PDF" }));
    await waitFor(() => expect(renderHistogramBundlePdf).toHaveBeenCalledTimes(2));
    expect(renderHistogramBundlePdf.mock.lastCall[0].includeComparisons).toBe(false);
  });

  test("reports export errors and restores the controls", async () => {
    renderHistogramBundlePdf.mockRejectedValue(new Error("Cannot merge bins across a gap."));
    render(<TablePanel state={{ panel_id: panelId, columns: ["Observable"], rows: [["energy"]],
      payload: { actions: { export: true }, histograms: { energy: { bins: [] } } } }} />);
    fireEvent.click(screen.getByRole("button", { name: "Download plots PDF" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Cannot merge bins across a gap");
    expect(screen.getByRole("button", { name: "Download plots PDF" })).toBeEnabled();
    expect(downloadBlob).not.toHaveBeenCalled();
  });
});
