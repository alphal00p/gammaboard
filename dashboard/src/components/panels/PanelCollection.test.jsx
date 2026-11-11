import { useState } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import PanelCollection from "./PanelCollection";

const { renderChart } = vi.hoisted(() => ({ renderChart: vi.fn() }));

vi.mock("./LazyChart", () => ({
  default: (props) => {
    renderChart(props);
    return <div data-testid="histogram-chart" />;
  },
}));

beforeEach(() => {
  renderChart.mockClear();
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const imageSpec = { panel_id: "pdf_adaptation_pdf", label: "Sampler PDF", kind: "image2d" };
const histogramSpec = { panel_id: "pdf_histogram", label: "PDF Histogram", kind: "histogram" };

test("image controls share the progress card and preserve their panel action", () => {
  const change = vi.fn();
  render(<PanelCollection panelSpecs={[
    { panel_id: "image_progress", kind: "progress", label: "Image Progress" },
    { panel_id: "image_view_mode", kind: "select", label: "Image component", state: {
      default_value: "component:x", options: [{ value: "component:x", label: "x" }, { value: "complex_phase", label: "Complex phase" }],
    } },
  ]} panelStates={[{ panel_id: "image_progress", kind: "progress", current: 4, total: 4 }]}
    onPanelValueChange={change} />);
  const input = screen.getByRole("combobox", { name: "Image component" });
  expect(input.closest(".MuiCard-root")).toBe(screen.getByText("Image Progress").closest(".MuiCard-root"));
  expect(input).toHaveTextContent("x");
  fireEvent.mouseDown(input);
  fireEvent.click(screen.getByRole("option", { name: "Complex phase" }));
  expect(change).toHaveBeenCalledWith("image_view_mode", "complex_phase", true);
});

test("phase images use a phase key and magnitude limits excluding invalid pixels", () => {
  render(<PanelCollection panelSpecs={[{ ...imageSpec, label: "Phase image" }]} panelStates={[{
    panel_id: imageSpec.panel_id, kind: "image2d", width: 5, height: 1,
    color_mode: "complex_phase", values: [1, -1, 0, 0, 1000000], imag_values: [0, 0, 1, -1, 1000000], invalid_indices: [4],
  }]} />);
  expect(screen.getByRole("img", { name: /^Complex phase colour key/ })).toBeInTheDocument();
  expect(screen.getByText("Magnitude |z|")).toBeInTheDocument();
  expect(screen.getByText("1.000e+0")).toBeInTheDocument();
  expect(screen.queryByText("-1.000e+0")).not.toBeInTheDocument();
});

test("keeps expanded campaign observables before Task Summary", () => {
  const specs = [
    { panel_id: "campaign_histograms", label: "Combined Observables", kind: "table", width: "full" },
    { panel_id: "task_summary", label: "Task Summary", kind: "key_value", width: "full" },
  ];
  render(<PanelCollection panelSpecs={specs} panelStates={[
    { panel_id: "campaign_histograms", kind: "table", columns: ["Observable"], rows: [["energy"]], payload: {
      expands_to: { kind: "histogram", source: "selected_row" },
      expanded_label: "Combined Observable",
      histograms: { energy: { bins: [{ start: 0, stop: 1, x: 0.5, value: 2, error: 0.2 }] } },
    } },
    { panel_id: "task_summary", kind: "key_value", entries: [{ key: "state", label: "State", value: "active" }] },
  ]} />);
  const observable = screen.getByText("energy", { selector: "h6" });
  expect(screen.getByText("Combined Observables").compareDocumentPosition(observable) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(observable.compareDocumentPosition(screen.getByText("Task Summary")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

const ObservableBundleHarness = ({ panelId, label, histogramTitle = "energy", bins = [{ start: 0, stop: 1, value: 2, error: 0.2 }] }) => {
  const [values, setValues] = useState({});
  return <PanelCollection
    panelSpecs={[{ panel_id: panelId, label, kind: "table", width: "full" }]}
    panelStates={[{
      panel_id: panelId, kind: "table", columns: ["Name", "Title"],
      rows: [["energy", histogramTitle]], row_keys: ["energy"], visible_column_indices: [1],
      payload: {
        expands_to: { kind: "histogram", source: "selected_row" },
        actions: { upload_bundle: true },
        histograms: { energy: { title: histogramTitle, bins } },
      },
    }]}
    panelValues={values}
    onPanelValueChange={(id, value) => setValues((previous) => ({ ...previous, [id]: value }))}
  />;
};

describe.each([
  { panelId: "gammaloop_histogram_bundle", label: "Observables" },
  { panelId: "campaign_histograms", label: "Combined Observables" },
])("$label bundle comparisons", ({ panelId, label }) => {
  test.each([true, false])("Add Histogram matches the visible table title to dropdown text (match present: %s)", async (hasMatch) => {
    const title = "Energy distribution";
    const bins = [{ start: 0, stop: 1, value: 2, error: 0.1 }];
    const histograms = {
      first: { title: "First reference", bins },
      "metadata-match": { title, bins },
      ...(hasMatch ? { energy: { title: "Different observable", bins }, [title]: { title: "Unrelated metadata", bins } } : {}),
    };
    render(<ObservableBundleHarness panelId={panelId} label={label} histogramTitle={title} />);
    fireEvent.change(screen.getByLabelText("Upload histogram bundle"), { target: { files: [{
      name: "reference.json", text: async () => JSON.stringify({ histograms }),
    }] } });
    await screen.findByRole("button", { name: "Remove reference.json", exact: true });
    fireEvent.click(screen.getByRole("button", { name: "Comparisons", exact: true }));
    // Clear any automatic selection so Add Histogram must choose a new entry.
    const remove = screen.queryByRole("button", { name: "Remove", exact: true });
    if (remove) fireEvent.click(remove);
    renderChart.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Add histogram", exact: true }));
    const expected = hasMatch ? title : "first";
    expect(screen.getAllByRole("combobox").some((select) => select.textContent === expected)).toBe(true);
    expect(renderChart.mock.calls.some(([props]) =>
      props.option.series.some((series) => series.name === `reference.json: ${expected}`),
    )).toBe(true);
  });

  test("gives repeated filenames separate comparison controls and rejects oversized files", async () => {
    render(<ObservableBundleHarness panelId={panelId} label={label} />);
    const input = screen.getByLabelText("Upload histogram bundle");
    const file = { name: "reference.json", text: async () => JSON.stringify({ histograms: { energy: { bins: [{ start: 0, stop: 1, value: 2, error: 0.1 }] } } }) };
    fireEvent.change(input, { target: { files: [file] } });
    await screen.findByRole("button", { name: "Remove reference.json", exact: true });
    fireEvent.change(input, { target: { files: [file] } });
    await screen.findByRole("button", { name: "Remove reference.json (2)", exact: true });
    expect(screen.getAllByRole("spinbutton", { name: "Merge adjacent bins", exact: true })).toHaveLength(1);
    expect(renderChart.mock.calls.at(-1)[0].option.series.some((series) => series.name.includes("reference.json (2)"))).toBe(true);
    const text = vi.fn();
    fireEvent.change(input, { target: { files: [{ name: "large.hwu", size: 16 * 1024 * 1024 + 1, text }] } });
    expect(await screen.findByText("Histogram bundles must be at most 16 MiB.")).toBeInTheDocument();
    expect(text).not.toHaveBeenCalled();
  });

  test("imports HwU through the server with busy state and keeps previous comparisons on failure", async () => {
    let finish;
    const fetch = vi.fn(() => new Promise((resolve) => { finish = resolve; }));
    vi.stubGlobal("fetch", fetch);
    render(<ObservableBundleHarness panelId={panelId} label={label} />);
    const input = screen.getByLabelText("Upload histogram bundle");
    const contents = '<histogram> 1 "energy"\n0 1 4 0.1\n<\\histogram>';
    fireEvent.change(input, { target: { files: [{ name: "reference.HwU", text: async () => contents }] } });
    await waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    expect(screen.getByRole("button", { name: "Importing bundle…" })).toBeDisabled();
    expect(fetch.mock.calls[0][0]).toBe("/api/histogram-bundle/import");
    expect(fetch.mock.calls[0][1]).toMatchObject({ method: "POST", body: contents, credentials: "include" });
    await act(async () => finish({ ok: true, json: async () => ({
      histograms: { energy: { bins: [{ start: 0, stop: 1, value: 4, error: 0.1 }] } }, warnings: ["Extra weight columns omitted."],
    }) }));
    expect(await screen.findByRole("button", { name: "Remove reference.HwU" })).toBeInTheDocument();
    expect(screen.getByText(/Extra weight columns omitted/)).toBeInTheDocument();
    fetch.mockResolvedValueOnce({ ok: false, status: 400, headers: { get: () => "application/json" }, json: async () => ({ error: "HwU line 2: invalid bin" }) });
    fireEvent.change(input, { target: { files: [{ name: "invalid.hwu", text: async () => "bad" }] } });
    expect(await screen.findByText(/HwU line 2: invalid bin/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove reference.HwU" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Upload Bundle" })).toBeEnabled();
  });

  test("shares display bin merging with references, keeps it on refresh, and resets from originals", async () => {
    const bins = [{ start: 0, stop: 1, value: 1, error: 0.3 }, { start: 1, stop: 2, value: 2, error: 0.4 }];
    const { rerender } = render(<ObservableBundleHarness panelId={panelId} label={label} bins={bins} />);
    const merge = screen.getByRole("spinbutton", { name: "Merge adjacent bins", exact: true });
    renderChart.mockClear();
    fireEvent.change(merge, { target: { value: "2" } });
    const chart = renderChart.mock.calls.find(([props]) => props.option.series.some((s) => s.name === "value"))?.[0];
    expect(chart.option.series.find((s) => s.name === "value").data).toEqual([[0, 3], [2, 3]]);
    fireEvent.click(screen.getByRole("button", { name: "energy information" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(/uncorrelated-bin approximation/);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    fireEvent.change(screen.getByLabelText("Upload histogram bundle"), { target: { files: [{
      name: "reference.json", text: async () => JSON.stringify({ histograms: { energy: { bins } } }),
    }] } });
    await screen.findByRole("button", { name: "Remove reference.json" });
    expect(renderChart.mock.calls.some(([props]) => props.option.series.some((series) => series.name === "reference.json: energy" && JSON.stringify(series.data) === JSON.stringify([[0, 3], [2, 3]])))).toBe(true);
    rerender(<ObservableBundleHarness panelId={panelId} label={label} bins={bins.map((bin) => ({ ...bin, value: bin.value * 2 }))} />);
    expect(merge).toHaveValue(2);
    fireEvent.click(screen.getByRole("button", { name: "Reset view", exact: true }));
    expect(merge).toHaveValue(1);
    expect(bins.map((bin) => bin.value)).toEqual([1, 2]);
  });

  test("uploads, selects and removes reference histograms through the shared panel", async () => {
    const { container, rerender } = render(<ObservableBundleHarness panelId={panelId} label={label} />);
    const input = container.querySelector('input[type="file"]');
    const openFile = vi.spyOn(input, "click");
    fireEvent.click(screen.getByRole("button", { name: "Upload Bundle" }));
    expect(openFile).toHaveBeenCalledOnce();
    fireEvent.change(input, { target: { files: [{
      name: "reference.json",
      text: async () => JSON.stringify({ histograms: {
        energy: {
          sample_count: 2,
          bins: [{ x_min: 0, x_max: 1, sum_weights: 8, sum_weights_squared: 34 }],
        },
        alternative: { bins: [{ start: 0, stop: 1, value: 3, error: 0.1 }] },
      } }),
    }] } });
    expect(await screen.findByRole("button", { name: "Remove reference.json" })).toBeInTheDocument();
    expect(renderChart.mock.calls.some(([props]) =>
      props.option.series.some((series) => series.name === "reference.json: energy"),
    )).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Comparisons", exact: true }));
    const selection = screen.getAllByRole("combobox").find((select) => select.textContent === "energy");
    fireEvent.mouseDown(selection);
    renderChart.mockClear();
    fireEvent.click(screen.getByRole("option", { name: "alternative", exact: true }));
    await waitFor(() => expect(renderChart.mock.calls.some(([props]) =>
      props.option.series.some((series) => series.name === "reference.json: alternative"),
    )).toBe(true));

    // Polling a new panel state must retain the uploaded bundle and its selection.
    rerender(<ObservableBundleHarness panelId={panelId} label={label} />);
    expect(screen.getAllByRole("combobox").some((select) => select.textContent === "alternative")).toBe(true);

    renderChart.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Remove reference.json" }));
    expect(screen.queryByRole("button", { name: "Remove reference.json" })).not.toBeInTheDocument();
    expect(renderChart.mock.calls.length).toBeGreaterThan(0);
    expect(renderChart.mock.calls.some(([props]) =>
      props.option.series.some((series) => series.name?.startsWith("reference.json:")),
    )).toBe(false);
  });

  test("reports invalid bundles and clears the error after a valid upload", async () => {
    const { container } = render(<ObservableBundleHarness panelId={panelId} label={label} />);
    const input = container.querySelector('input[type="file"]');
    fireEvent.change(input, { target: { files: [{ name: "invalid.json", text: async () => "{}" }] } });
    expect(await screen.findByText("bundle payload must contain a 'histograms' object")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove invalid.json" })).not.toBeInTheDocument();

    fireEvent.change(input, { target: { files: [{
      name: "valid.json",
      text: async () => JSON.stringify({ histograms: { energy: {
        bins: [{ start: 0, stop: 1, value: 3, error: 0.1 }],
      } } }),
    }] } });
    expect(await screen.findByRole("button", { name: "Remove valid.json" })).toBeInTheDocument();
    expect(screen.queryByText("bundle payload must contain a 'histograms' object")).not.toBeInTheDocument();
  });
});

const pdfImageSpecs = ["log_integrand", "log_pdf", "oversampling"].map((name) => ({
  panel_id: `pdf_adaptation_${name}`,
  label: name,
  kind: "image2d",
}));
const pdfImageState = (spec, normalizationMode, metricMode = "log10_ratio", dataValues = [-8, -2, 0, 1]) => ({
  panel_id: spec.panel_id,
  kind: spec.kind,
  width: 2,
  height: 2,
  values: dataValues,
  normalization_mode: normalizationMode ?? (spec.panel_id.endsWith("oversampling") ? "zero_centered" : "linear"),
  metric_mode: metricMode,
});
const HeatmapHarness = ({ normalizationMode, metricMode = "log10_ratio", dataValues }) => {
  const [values, setValues] = useState({});
  return (
    <PanelCollection
      panelSpecs={pdfImageSpecs}
      panelStates={pdfImageSpecs.map((spec) => pdfImageState(spec, normalizationMode, metricMode, dataValues))}
      panelValues={values}
      onPanelValueChange={(id, value) => setValues((previous) => ({ ...previous, [id]: value }))}
    />
  );
};

describe("Image heatmap controls", () => {
  test.each([undefined, "zero_centered", "symmetric"])("uses independent data limits for %s payloads", (normalizationMode) => {
    render(<HeatmapHarness normalizationMode={normalizationMode} />);
    expect(screen.queryByText("Spread")).not.toBeInTheDocument();
    expect(screen.getAllByRole("combobox", { name: /^Color scale/ }).map((select) => select.textContent))
      .toEqual(["linear", "linear", "zero-origin"]);
    const minInputs = screen.getAllByRole("spinbutton", { name: "Min color limit" });
    const maxInputs = screen.getAllByRole("spinbutton", { name: "Max color limit" });
    expect(minInputs).toHaveLength(3);
    expect(maxInputs).toHaveLength(3);
    minInputs.forEach((input) => expect(input).toHaveValue(-8));
    maxInputs.forEach((input) => expect(input).toHaveValue(1));

    fireEvent.change(minInputs[0], { target: { value: "-4" } });
    fireEvent.blur(minInputs[0]);
    expect(minInputs[0]).toHaveValue(-4);
    expect(maxInputs[0]).toHaveValue(1);
    expect(minInputs[1]).toHaveValue(-8);

    fireEvent.change(maxInputs[0], { target: { value: "2" } });
    fireEvent.blur(maxInputs[0]);
    expect(minInputs[0]).toHaveValue(-4);
    expect(maxInputs[0]).toHaveValue(2);
    expect(maxInputs[1]).toHaveValue(1);

    fireEvent.click(screen.getAllByRole("button", { name: "Auto limits" })[0]);
    expect(screen.getAllByRole("spinbutton", { name: "Min color limit" })[0]).toHaveValue(-8);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(1);
  });

  test("rejects limits that would exclude zero and resets bounds on a metric switch", () => {
    const { rerender } = render(<HeatmapHarness />);
    const max = screen.getAllByRole("spinbutton", { name: "Max color limit" })[2];
    fireEvent.change(max, { target: { value: "-1" } });
    fireEvent.blur(max);
    expect(screen.getByText("Must be ≥ 0 and above Min.")).toBeInTheDocument();
    expect(screen.getAllByRole("slider", { name: "Max color limit slider" })[2]).toHaveAttribute("aria-valuenow", "1");

    fireEvent.change(max, { target: { value: "0.25" } });
    fireEvent.blur(max);
    expect(screen.queryByText("Must be ≥ 0 and above Min.")).not.toBeInTheDocument();
    rerender(<HeatmapHarness metricMode="relative_mismatch" />);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[2]).toHaveValue(1);
  });

  test("switches scales per plot with fresh bounds for one-sided data", () => {
    render(<HeatmapHarness dataValues={[-11, -10, -9, -8]} />);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(-8);
    const max = screen.getAllByRole("spinbutton", { name: "Max color limit" })[0];
    fireEvent.change(max, { target: { value: "-9" } });
    fireEvent.blur(max);
    expect(max).toHaveValue(-9);
    expect(screen.getAllByRole("slider", { name: "Max color limit slider" })[0]).toHaveAttribute("aria-valuenow", "-9");

    fireEvent.mouseDown(screen.getAllByRole("combobox", { name: /^Color scale/ })[0]);
    fireEvent.click(screen.getByRole("option", { name: "zero-origin" }));
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(0);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[1]).toHaveValue(-8);
    fireEvent.mouseDown(screen.getAllByRole("combobox", { name: /^Color scale/ })[0]);
    fireEvent.click(screen.getByRole("option", { name: "linear" }));
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(-8);
  });

  test("accepts positive linear minima, rejects crossed bounds, and keeps the scale on Auto limits", () => {
    render(<HeatmapHarness dataValues={[2, 3, 4, 8]} />);
    const min = screen.getAllByRole("spinbutton", { name: "Min color limit" })[0];
    const max = screen.getAllByRole("spinbutton", { name: "Max color limit" })[0];
    expect(min).toHaveValue(2);
    fireEvent.change(min, { target: { value: "4" } });
    fireEvent.blur(min);
    expect(screen.getAllByRole("slider", { name: "Min color limit slider" })[0]).toHaveAttribute("aria-valuenow", "4");
    fireEvent.change(max, { target: { value: "4" } });
    fireEvent.blur(max);
    expect(screen.getByText("Must be above Min.")).toBeInTheDocument();
    expect(screen.getAllByRole("slider", { name: "Max color limit slider" })[0]).toHaveAttribute("aria-valuenow", "8");
    fireEvent.click(screen.getAllByRole("button", { name: "Auto limits" })[0]);
    expect(screen.getAllByRole("spinbutton", { name: "Min color limit" })[0]).toHaveValue(2);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(8);
    expect(screen.queryByText("Must be above Min.")).not.toBeInTheDocument();
    expect(screen.getAllByRole("combobox", { name: /^Color scale/ })[0]).toHaveTextContent("linear");
  });

  test("changes a color slider without changing either its other limit or another plot's limits", () => {
    render(<HeatmapHarness />);
    fireEvent.change(screen.getAllByRole("slider", { name: "Min color limit slider" })[0], { target: { value: "-4" } });
    expect(screen.getAllByRole("spinbutton", { name: "Min color limit" })[0]).toHaveValue(-4);
    expect(screen.getAllByRole("spinbutton", { name: "Max color limit" })[0]).toHaveValue(1);
    expect(screen.getAllByRole("spinbutton", { name: "Min color limit" })[1]).toHaveValue(-8);
  });


});

const renderTimeseries = ({ kind = "scalar_timeseries", panelId = "test_history", xAxis, points, value, target, xRange, yRange }) => {
  const state = kind === "scalar_timeseries"
    ? { points, target }
    : { series: [{ id: "test", label: "Test", points }] };
  render(
    <PanelCollection
      panelSpecs={[{ panel_id: panelId, label: "Test", kind, history: "append" }]}
      panelStates={[{ ...state, panel_id: panelId, x_axis: xAxis, x_range: xRange, y_range: yRange }]}
      panelValues={{ [panelId]: value }}
    />,
  );
  return renderChart.mock.lastCall[0].option;
};

const tooltipHeader = (option, x) => option.tooltip.formatter([
  { axisValue: x, seriesName: "Test", value: [x, 0.8] },
]).split("<br/>")[0];

test("busy graphs preserve gaps, measured zero, and shared fixed axes", () => {
  const option = renderTimeseries({
    kind: "multi_timeseries", xAxis: "wall_time", xRange: [1000, 61000], yRange: [0, 100],
    points: [{ x: 2000, y: 50 }, { x: 3000, y: 50 }, { x: 8000, y: 0, break_before: true }, { x: 9000, y: 0 }],
  });
  expect([option.xAxis.min, option.xAxis.max]).toEqual([1000, 61000]);
  expect([option.yAxis.min, option.yAxis.max]).toEqual([0, 100]);
  const line = option.series.find((series) => series.type === "line");
  expect(line.connectNulls).toBe(false);
  expect(line.smooth).toBe(false);
  expect(line.data).toEqual([[2000, 50], [3000, 50], [8000, null], [8000, 0], [9000, 0]]);
  expect(option.tooltip.formatter([{ axisValue: 8000, seriesName: "Test", value: [8000, null] }])).toContain("n/a");
});

test("subsecond graph timestamps remain distinguishable when zoomed", () => {
  const origin = 1700000000000;
  const option = renderTimeseries({ kind: "multi_timeseries", xAxis: "wall_time", xRange: [origin, origin + 1000], points: [{ x: origin, y: 0 }, { x: origin + 500, y: 80 }] });
  expect(option.xAxis.axisLabel.formatter(origin + 500)).toBe("00:00.500");
  expect(tooltipHeader(option, origin + 500)).toBe("00:00.500");
});

test("axis tooltips omit helper series and replace old traces on refresh", () => {
  const option = renderTimeseries({ kind: "multi_timeseries", xAxis: "wall_time", points: [{ x: 1000, y: 40, y_min: 0, y_max: 80 }] });
  const tooltip = option.tooltip.formatter([
    { seriesType: "custom", seriesName: "Test", seriesId: "range", axisValue: 1000, value: [1000, 0, 80] },
    { seriesType: "line", seriesName: "Test", seriesId: "test", axisValue: 1000, value: [1000, 40] },
    { seriesType: "line", seriesName: "Test", seriesId: "test", axisValue: 1000, value: [1000, 40] },
  ]);
  expect(tooltip.match(/Test:/g)).toHaveLength(1);
  expect(renderChart.mock.lastCall[0].replaceMerge).toEqual(["series"]);
});

describe("PanelCollection scalar legends", () => {
  test.each(["ess_history", "abs_signal_to_noise_history"])("omits empty uncertainty and the single-series legend for %s", (panelId) => {
    const option = renderTimeseries({
      panelId,
      xAxis: "completed_samples",
      points: [{ x: 250, y: 0.2 }, { x: 1000, y: 0.8 }],
    });
    expect(option.legend.show).toBe(false);
    expect(option.series).toHaveLength(1);
    expect(option.series[0].name).toBe("Test");
    expect(option.grid.top).toBe(12);
    expect(option.xAxis.nameLocation).toBe("middle");
    expect(renderChart.mock.lastCall[0].replaceMerge).toEqual(["series"]);
  });

  test("does not interpret null bounds or a null target as zero-valued data", () => {
    const option = renderTimeseries({
      points: [{ x: 250, y: -2, y_min: -3, y_max: null }, { x: 1000, y: -1, y_min: null, y_max: null }],
      target: null,
    });
    expect(option.legend.show).toBe(false);
    expect(option.series).toHaveLength(1);
  });

  test("preserves real uncertainty bands and target lines for mean histories", () => {
    const option = renderTimeseries({
      panelId: "real_estimate_history",
      xAxis: "completed_samples",
      points: [{ x: 250, y: 0.2, y_min: 0.1, y_max: 0.3 }, { x: 1000, y: 0.8, y_min: 0.7, y_max: 0.9 }],
      target: 0,
    });
    expect(option.legend.show).toBe(true);
    expect(option.series.map((series) => series.name)).toEqual(["uncertainty", "Test", "target"]);
    expect(option.series[0].data).toEqual([[250, 0.1, 0.3, 1000, 0.7, 0.9]]);
    expect(option.series[0].progressive).toBe(0);
    expect(option.series[2].data).toEqual([[250, 0], [1000, 0]]);
  });

  test("shows an error bar for the first mean point before a band can be drawn", () => {
    const option = renderTimeseries({
      panelId: "real_estimate_history",
      points: [{ x: 250, y: 0.2, y_min: 0.1, y_max: 0.3 }],
    });
    expect(option.legend.show).toBe(true);
    expect(option.series[0].name).toBe("uncertainty");
    expect(option.series[0].data).toEqual([[250, 0.1, 0.3]]);
  });
});

describe("PanelCollection timeseries axes", () => {
  test.each([
    "real_estimate_history",
    "imag_estimate_history",
    "abs_signal_to_noise_history",
    "ess_history",
  ])("plots %s against numeric completed samples", (panelId) => {
    const option = renderTimeseries({
      panelId,
      xAxis: "completed_samples",
      points: [{ x: 1000, y: 0.2 }, { x: 1e12, y: 0.8 }],
    });

    expect(option.xAxis.name).toBe("Completed Samples");
    expect(option.xAxis.axisLabel.formatter(1000)).toBe("1.000e+3");
    expect(option.xAxis.axisLabel.formatter(1e12)).toBe("1.000e+12");
    expect(tooltipHeader(option, 1000)).toBe("1.000e+3");
    expect(option.series.find((series) => series.type === "line").data).toEqual([[1000, 0.2], [1e12, 0.8]]);
    expect(option.yAxis.axisLabel.formatter(0.8)).toBe("8.000e-1");
  });

  describe.each(["scalar_timeseries", "multi_timeseries"])("%s", (kind) => {
    test.each(["sampler_uptime", "wall_time"])("ignores a stale %s preference for sample plots", (xAxisMode) => {
      const option = renderTimeseries({
        kind,
        xAxis: "completed_samples",
        points: [{ x: 1000, y: 0.8 }],
        value: { xAxisMode },
      });
      expect(option.xAxis.name).toBe("Completed Samples");
      expect(option.xAxis.nameLocation).toBe("middle");
      expect(option.xAxis.nameGap).toBe(30);
      expect(tooltipHeader(option, 1000)).toBe("1.000e+3");
    });

    test("keeps ordinary numeric coordinates numeric regardless of size or panel name", () => {
      const option = renderTimeseries({ kind, points: [{ x: 1e12, y: 0.8 }] });
      expect(option.xAxis.name).toBe("x");
      expect(option.xAxis.axisLabel.formatter(1e12)).toBe("1.000e+12");
    });

    test.each([
      { mode: undefined, label: "Sampler Runner Uptime", xs: [1000, 61000], tick: "01:01" },
      { mode: "sampler_uptime", label: "Sampler Runner Uptime", xs: [1000, 61000], tick: "01:01" },
      { mode: "wall_time", label: "Elapsed Time", xs: [1700000000000, 1700000060000], tick: "01:00" },
      { mode: "completed_samples", label: "Completed Samples", xs: [250, 1000], tick: "1.000e+3" },
    ])("preserves performance axis mode $mode", ({ mode, label, xs, tick }) => {
      const option = renderTimeseries({
        kind,
        xAxis: "wall_time",
        points: [
          { x: 1700000000000, x_sampler_uptime_ms: 1000, x_completed_samples_total: 250, y: 0.2 },
          { x: 1700000060000, x_sampler_uptime_ms: 61000, x_completed_samples_total: 1000, y: 0.8 },
        ],
        value: { xAxisMode: mode },
      });
      expect(option.xAxis.name).toBe(label);
      expect(option.series.find((series) => series.type === "line").data.map(([x]) => x)).toEqual(xs);
      expect(option.xAxis.axisLabel.formatter(xs[1])).toBe(tick);
      expect(tooltipHeader(option, xs[1])).toBe(tick);
    });

    test.each(["sampler_uptime", "completed_samples"])("never substitutes wall time for missing %s metadata", (xAxisMode) => {
      const option = renderTimeseries({
        kind,
        xAxis: "wall_time",
        points: [
          { x: 1700000000000, y: 0.2 },
          { x: 1700000060000, x_sampler_uptime_ms: 61000, x_completed_samples_total: 1000, y: 0.8 },
          { x: 1700000120000, x_sampler_uptime_ms: null, x_completed_samples_total: null, y: 0.9 },
        ],
        value: { xAxisMode },
      });
      const x = xAxisMode === "sampler_uptime" ? 61000 : 1000;
      expect(option.series.find((series) => series.type === "line").data).toEqual([[x, 0.8]]);
    });

    test("uses wall time when uptime coordinates are unavailable", () => {
      const option = renderTimeseries({
        kind,
        xAxis: "wall_time",
        points: [{ x: 1700000000000, y: 0.2 }, { x: 1700000060000, y: 0.8 }],
        value: { xAxisMode: "sampler_uptime" },
      });
      expect(option.xAxis.name).toBe("Elapsed Time");
      expect(option.xAxis.axisLabel.formatter(1700000060000)).toBe("01:00");
    });
  });
});

describe("PanelCollection plot availability", () => {
  test.each([imageSpec, histogramSpec])("keeps $kind panels visible without an update", (spec) => {
    render(<PanelCollection panelSpecs={[spec]} panelStates={[]} />);

    expect(screen.getByText(spec.label)).toBeInTheDocument();
    expect(screen.getByText("No plot data available.")).toBeInTheDocument();
  });

  test.each([
    { spec: imageSpec, state: { width: 128, height: 128, values: [] } },
    { spec: histogramSpec, state: { bins: [] } },
  ])("keeps empty $spec.kind payloads visible", ({ spec, state }) => {
    render(
      <PanelCollection panelSpecs={[spec]} panelStates={[{ ...state, panel_id: spec.panel_id }]} />,
    );

    expect(screen.getByText(spec.label)).toBeInTheDocument();
    expect(screen.getByText("No plot data available.")).toBeInTheDocument();
  });

  test("replaces missing-data messages when image and histogram updates arrive", () => {
    const panelSpecs = [imageSpec, histogramSpec];
    const { container, rerender } = render(<PanelCollection panelSpecs={panelSpecs} panelStates={[]} />);
    expect(screen.getAllByText("No plot data available.")).toHaveLength(2);

    rerender(
      <PanelCollection
        panelSpecs={panelSpecs}
        panelStates={[
          {
            panel_id: imageSpec.panel_id,
            width: 2,
            height: 2,
            values: [9.82e-18, 1e-12, 1e-11, 7.19e-10],
            x_range: [0, 1],
            y_range: [0, 1],
          },
          {
            panel_id: histogramSpec.panel_id,
            controls: { default_relative_error: false },
            bins: [
              { start: 0, stop: 0.5, value: 0.25 },
              { start: 0.5, stop: 1, value: 0.75 },
            ],
          },
        ]}
      />,
    );

    expect(screen.queryByText("No plot data available.")).not.toBeInTheDocument();
    expect(screen.getByText(imageSpec.label)).toBeInTheDocument();
    expect(container.querySelector("canvas")).toBeInTheDocument();
    expect(screen.getByText(histogramSpec.label)).toBeInTheDocument();
    expect(screen.getAllByTestId("histogram-chart")).toHaveLength(1);
  });
});

const StratumHarness = ({state}) => {
  const [values,setValues]=useState({});
  return <PanelCollection panelSpecs={[{panel_id:state.panel_id,kind:state.kind,label:'Diagnostics',width:'full'}]} panelStates={[state]} panelValues={values} onPanelValueChange={(id,value)=>setValues(previous=>({...previous,[id]:value}))}/>;
};

test('stratum component and history controls update their own panel state',()=>{
  const state={kind:'strata',panel_id:'strata',estimator:'stratified',components:['x','y'],frames:[{samples:10,total:[{mean:1,error:0.1},{mean:2,error:0.2}],rows:[{prefix:[1,3],attempts:10,matches:8,probability:1,estimates:[{mean:1,error:0.1},{mean:2,error:0.2}]}]}]};
  render(<StratumHarness state={state}/>);
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^Component/}));
  fireEvent.click(screen.getByRole('option',{name:'y',exact:true}));
  expect(screen.getByText('Share of reported variance (y)')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Total y' })).toBeInTheDocument();
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^History/}));
  fireEvent.click(screen.getByRole('option',{name:'Hit rate',exact:true}));
  expect(screen.getByRole('checkbox',{name:'Between snapshots'})).toBeInTheDocument();
});

test('PDF normalization and stratum controls update without discarding the other view settings',()=>{
  const state={kind:'pdf_comparison',panel_id:'pdf_comparison',geometry:{kind:'line',geometry:{linspace:{start:0,stop:1,count:2}}},data:{integrand_values:[-1,1],pdf_values:[1,1],strata:{prefixes:[[0],[1,3]],probabilities:[0.25,0.75],actual_strata:[0,1],pdf_values:[[0.4,0.1],[0.6,0.9]]}}};
  render(<StratumHarness state={state}/>);
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^Normalization/}));
  fireEvent.click(screen.getByRole('option',{name:'Conditional density',exact:true}));
  expect(screen.getByText(/The PDF is raw/)).toBeInTheDocument();
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^Scope/}));
  fireEvent.click(screen.getByRole('option',{name:'Stratum (1,3)',exact:true}));
  expect(screen.getByRole('combobox',{name:/^Normalization/})).toHaveTextContent('Conditional density');
  expect(screen.queryByText(/^Stratum \(0\) · p/)).not.toBeInTheDocument();
  const curves=renderChart.mock.calls.at(-1)[0].option.series;
  expect(curves.find(s=>s.name==='Sampler PDF').data[0][1]).toBeCloseTo(0.8);
});

test('a zero slice target is explained and omitted only when unit-area normalization is undefined', () => {
  const state = {
    kind: 'pdf_comparison', panel_id: 'pdf_comparison',
    geometry: {kind: 'line', geometry: {linspace: {start: 0, stop: 1, count: 2}}},
    data: {integrand_values: [1, 2], pdf_values: [1, 1], strata: {
      prefixes: [[0,0], [0,1]], probabilities: [0.5, 0.5],
      actual_strata: [0, 0], pdf_values: [[0.5, 0.5], [0.5, 0.5]],
    }},
  };
  render(<StratumHarness state={state} />);
  fireEvent.mouseDown(screen.getByRole('combobox', {name: /^Scope/}));
  fireEvent.click(screen.getByRole('option', {name: 'Stratum (0,1)', exact: true}));
  expect(screen.getByRole('alert')).toHaveTextContent('This stratum has no target support at the evaluated slice points');
  expect(renderChart.mock.calls.at(-1)[0].option.series.map(s => s.name)).toEqual(['Sampler PDF']);
  fireEvent.mouseDown(screen.getByRole('combobox', {name: /^Normalization/}));
  fireEvent.click(screen.getByRole('option', {name: 'Joint density', exact: true}));
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  const target = renderChart.mock.calls.at(-1)[0].option.series.find(s => s.name === 'Target');
  expect(target.data).toEqual([[0, 0], [1, 0]]);
});

test('many strata start with one overview and comparisons are paged four at a time',()=>{
  const state={kind:'pdf_comparison',panel_id:'pdf_comparison',geometry:{kind:'line',geometry:{linspace:{start:0,stop:1,count:2}}},data:{integrand_values:[1,1],pdf_values:[1,1],strata:{prefixes:Array.from({length:11},(_,i)=>[i]),probabilities:Array(11).fill(1/11),actual_strata:[0,10],pdf_values:Array.from({length:11},()=>[1/11,1/11])}}};
  render(<StratumHarness state={state}/>);
  expect(screen.getAllByTestId('histogram-chart')).toHaveLength(1);
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^Scope/}));
  fireEvent.click(screen.getByRole('option',{name:'Compare strata',exact:true}));
  expect(screen.getAllByTestId('histogram-chart')).toHaveLength(4);
  expect(screen.getByText('1–4 of 11')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'Next',exact:true}));
  expect(screen.getByText('5–8 of 11')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'Next',exact:true}));
  expect(screen.getAllByTestId('histogram-chart')).toHaveLength(3);
  expect(screen.getByRole('button',{name:'Next',exact:true})).toBeDisabled();
});

test('ordinary PDF heatmaps share logarithmic limits, manual limits and persisted zoom',()=>{
  const state={kind:'pdf_comparison',panel_id:'pdf_comparison',geometry:{kind:'plane',geometry:{u_linspace:{start:0,stop:1,count:2},v_linspace:{start:0,stop:1,count:2}}},data:{integrand_values:[0.000001,1,2,10],pdf_values:[1,2,3,4],strata:null}};
  render(<StratumHarness state={state}/>);
  expect(screen.queryByRole('combobox',{name:/^Scope/})).not.toBeInTheDocument();
  fireEvent.mouseDown(screen.getByRole('combobox',{name:/^Density scale/}));
  fireEvent.click(screen.getByRole('option',{name:'Logarithmic',exact:true}));
  const options=renderChart.mock.calls.slice(-2).map(c=>c[0].option);
  expect(options[0].visualMap.min).toBe(options[1].visualMap.min);
  expect(options[0].visualMap.max).toBe(options[1].visualMap.max);
  act(()=>renderChart.mock.calls.at(-1)[0].onEvents.datazoom({dataZoomId:'x-slider',start:25,end:75}));
  expect(renderChart.mock.calls.slice(-2).every(c=>c[0].option.dataZoom[0].start===25)).toBe(true);
  fireEvent.click(screen.getByText('Colour limits · linked across comparable plots'));
  fireEvent.change(screen.getByRole('spinbutton',{name:'Min color limit'}),{target:{value:'-4'}});
  fireEvent.blur(screen.getByRole('spinbutton',{name:'Min color limit'}));
  expect(renderChart.mock.calls.slice(-2).every(c=>c[0].option.visualMap.min===-4)).toBe(true);
  expect(renderChart.mock.calls.at(-1)[0].option.dataZoom[0].start).toBe(25);
  fireEvent.click(screen.getByRole('button',{name:'Reset view',exact:true}));
  expect(renderChart.mock.calls.at(-1)[0].option.dataZoom[0].start).toBe(0);
});

test('large prefix lists can be searched without rendering all stratum plots',()=>{
  const state={kind:'pdf_comparison',panel_id:'pdf_comparison',geometry:{kind:'line',geometry:{linspace:{start:0,stop:1,count:2}}},data:{integrand_values:[1,1],pdf_values:[1,1],strata:{prefixes:Array.from({length:30},(_,i)=>[i]),probabilities:Array(30).fill(1/30),actual_strata:[0,29],pdf_values:Array.from({length:30},()=>[1/30,1/30])}}};
  render(<StratumHarness state={state}/>);
  const scope=screen.getByRole('combobox',{name:'Scope'});
  fireEvent.change(scope,{target:{value:'(29)'}});
  fireEvent.keyDown(scope,{key:'ArrowDown'});
  fireEvent.click(screen.getByRole('option',{name:'Stratum (29)',exact:true}));
  expect(screen.getAllByTestId('histogram-chart')).toHaveLength(1);
  expect(screen.getByText(/^Stratum \(29\) · p/)).toBeInTheDocument();
});

test('stratum hue is optional for 2D densities and survives view changes', () => {
  const state = {
    kind: 'pdf_comparison', panel_id: 'pdf_comparison',
    geometry: {kind: 'plane', geometry: {
      u_linspace: {start: 0, stop: 1, count: 2}, v_linspace: {start: 0, stop: 1, count: 2},
    }},
    data: {integrand_values: [1, -1, -1, 1], pdf_values: [1, 1, 1, 1], strata: {
      prefixes: [[0], [1]], probabilities: [0.5, 0.5], actual_strata: [1, 0, 0, 1],
      pdf_values: [[0.5, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]],
    }},
  };
  render(<StratumHarness state={state} />);
  expect(screen.getByRole('combobox', {name: /^Colour encoding/})).toHaveTextContent('Density only');
  fireEvent.mouseDown(screen.getByRole('combobox', {name: /^Colour encoding/}));
  fireEvent.click(screen.getByRole('option', {name: 'Stratum hue + density', exact: true}));
  expect(screen.getByText(/Hue shows the actual integrand stratum/)).toBeInTheDocument();
  const plots = renderChart.mock.calls.slice(-2).map(([props]) => props.option);
  expect(plots.every(plot => plot.series[0].data[0].value[4] === 1)).toBe(true);
  expect(plots.every(plot => plot.series[0].data[0].itemStyle.color.startsWith('rgb('))).toBe(true);
  fireEvent.mouseDown(screen.getByRole('combobox', {name: /^View/}));
  fireEvent.click(screen.getByRole('option', {name: 'Coverage', exact: true}));
  expect(screen.queryByRole('combobox', {name: /^Colour encoding/})).not.toBeInTheDocument();
  fireEvent.mouseDown(screen.getByRole('combobox', {name: /^View/}));
  fireEvent.click(screen.getByRole('option', {name: 'Density comparison', exact: true}));
  expect(screen.getByRole('combobox', {name: /^Colour encoding/})).toHaveTextContent('Stratum hue + density');
});

test('unchanged PDF data is not redrawn when its parent refreshes', () => {
  const state = {kind:'pdf_comparison',panel_id:'pdf_comparison',geometry:{kind:'plane',geometry:{u_linspace:{start:0,stop:1,count:2},v_linspace:{start:0,stop:1,count:2}}},data:{integrand_values:[1,2,3,4],pdf_values:[1,2,3,4],strata:null}};
  const props = {
    panelSpecs: [{panel_id:'pdf_comparison',kind:'pdf_comparison',label:'PDF',width:'full'}],
    panelStates: [state], panelValues: {}, onPanelValueChange: vi.fn(),
  };
  const { rerender } = render(<PanelCollection {...props} title="First poll" />);
  const draws = renderChart.mock.calls.length;
  rerender(<PanelCollection {...props} title="Second poll" />);
  expect(renderChart.mock.calls.length).toBe(draws);
  expect(screen.queryByRole('checkbox', {name:'Show stratum boundaries'})).not.toBeInTheDocument();
});

test("RSD fits the visible x range and selected components, retaining legend choices on refresh", () => {
  let refresh;
  const points = (a, b, c) => [{ x: 0, y: a }, { x: 50, y: b }, { x: 100, y: c }];
  function Harness() {
    const [values, setValues] = useState({});
    const [end, setEnd] = useState(12);
    refresh = () => setEnd(14);
    return <PanelCollection panelSpecs={[{ panel_id: "rsd_history", label: "RSD", kind: "multi_timeseries", history: "append" }]}
      panelStates={[{ panel_id: "rsd_history", kind: "multi_timeseries", x_axis: "completed_samples", series: [
        { id: "real", label: "Real", points: points(5000, 10, end) },
        { id: "imag", label: "Imag", points: points(10000, 100, 200) },
      ] }]} panelValues={values} onPanelValueChange={(id, value) => setValues((prev) => ({ ...prev, [id]: value }))} />;
  }
  render(<Harness />);
  const chart = () => renderChart.mock.lastCall[0];
  expect(chart().option.yAxis.max).toBeGreaterThan(10000);
  act(() => chart().onEvents.datazoom({ dataZoomId: "x-slider", start: 50, end: 100 }));
  expect(chart().option.yAxis.max).toBeLessThan(220);
  act(() => chart().onEvents.legendselectchanged({ selected: { Real: true, Imag: false } }));
  expect(chart().option.yAxis.min).toBeCloseTo(9.84);
  expect(chart().option.yAxis.max).toBeCloseTo(12.16);
  act(() => refresh());
  expect(chart().option.legend.selected.Imag).toBe(false);
  expect(chart().option.yAxis.max).toBeCloseTo(14.32);
  act(() => chart().onEvents.legendselectchanged({ selected: { Real: true, Imag: true } }));
  expect(chart().option.yAxis.max).toBeGreaterThan(200);
  act(() => chart().onEvents.legendselectchanged({ selected: { Real: false, Imag: false } }));
  expect(chart().option.yAxis.min).toBe("auto");
  expect(chart().option.yAxis.max).toBe("auto");
});

test("mean axes exclude hidden target and uncertainty legend entries", () => {
  renderTimeseries({ panelId: "mean_history", xAxis: "completed_samples", target: 1000,
    points: [{ x: 0, y: 10, y_min: -100, y_max: 100 }, { x: 100, y: 12, y_min: -80, y_max: 90 }] });
  const chart = () => renderChart.mock.lastCall[0];
  act(() => chart().onEvents.legendselectchanged({ selected: { target: false } }));
  expect(chart().option.yAxis.max).toBe(116);
  act(() => chart().onEvents.legendselectchanged({ selected: { target: false, uncertainty: false } }));
  expect(chart().option.yAxis.min).toBeCloseTo(9.84);
  expect(chart().option.yAxis.max).toBeCloseTo(12.16);
});

test("matched references and the comparison plot appear by default; explicit removal survives revisiting", async () => {
  const bins = [{ start: 0, stop: 1, value: 2, error: .1 }, { start: 1, stop: 2, value: 4, error: .2 }];
  function Harness() {
    const [values, setValues] = useState({});
    return <PanelCollection panelSpecs={[{ panel_id: "obs", label: "Observables", kind: "table" }]}
      panelStates={[{ panel_id: "obs", kind: "table", columns: ["Name"], rows: [["energy"], ["angle"]], row_keys: ["energy", "angle"],
        payload: { expands_to: { kind: "histogram", source: "selected_row" }, actions: { upload_bundle: true },
          histograms: { energy: { title: "Energy", bins }, angle: { title: "Angle", bins } } } }]}
      panelValues={values} onPanelValueChange={(id, value) => setValues((previous) => ({ ...previous, [id]: value }))} />;
  }
  render(<Harness />);
  expect(screen.queryByRole("table")).not.toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Observable" })).toHaveValue("Energy");
  fireEvent.change(screen.getByLabelText("Upload histogram bundle"), { target: { files: [{ name: "reference.json", text: async () => JSON.stringify({
    histograms: { different_key: { title: "Energy", bins }, Angle: { bins } },
  }) }] } });
  await screen.findByRole("button", { name: "Remove reference.json" });
  expect(screen.getByRole("button", { name: "Comparison", exact: true })).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByText("Comparison: 1 − result / reference · agreement at 0")).toBeVisible();
  expect(renderChart.mock.calls.some(([props]) => props.option.series.some((series) => series.name === "reference.json: different_key"))).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Comparison", exact: true }));
  expect(screen.queryByText("Comparison: 1 − result / reference · agreement at 0")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Comparisons", exact: true }));
  fireEvent.click(screen.getByRole("button", { name: "Remove", exact: true }));
  fireEvent.click(screen.getByRole("button", { name: "Next observable" }));
  expect(screen.getByRole("combobox", { name: "Observable" })).toHaveValue("Angle");
  expect(screen.getByRole("button", { name: "Comparison", exact: true })).toHaveAttribute("aria-pressed", "false");
  renderChart.mockClear();
  fireEvent.click(screen.getByRole("button", { name: "Previous observable" }));
  expect(screen.getByRole("combobox", { name: "Observable" })).toHaveValue("Energy");
  expect(screen.queryByRole("button", { name: "Comparison", exact: true })).not.toBeInTheDocument();
  expect(renderChart.mock.calls.some(([props]) => props.option.series.some((series) => series.name?.startsWith("reference.json:")))).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "All observables (2)" }));
  expect(screen.getByRole("table")).toBeVisible();
  fireEvent.click(screen.getByRole("cell", { name: "angle", exact: true }));
  expect(screen.getByRole("combobox", { name: "Observable" })).toHaveValue("Angle");
});

test("shared bin merging respects the shortest comparison and density input without changing originals", async () => {
  const bins = [0, 1, 2, 3].map((start) => ({ start, stop: start + 1, value: 1, error: .1 }));
  render(<ObservableBundleHarness panelId="obs" label="Observables" bins={bins} />);
  fireEvent.change(screen.getByRole("spinbutton", { name: "Merge adjacent bins" }), { target: { value: "4" } });
  const comparisonBins = [{ start: 0, stop: 2, value: 2, error: .2 }, { start: 2, stop: 4, value: 3, error: .3 }];
  fireEvent.change(screen.getByLabelText("Upload histogram bundle"), { target: { files: [{ name: "density.json", text: async () => JSON.stringify({ histograms: { energy: { bins: comparisonBins } } }) }] } });
  await screen.findByRole("button", { name: "Remove density.json" });
  const merge = screen.getByRole("spinbutton", { name: "Merge adjacent bins" });
  await waitFor(() => expect(merge).toHaveValue(2));
  expect(merge).toHaveAttribute("max", "2");
  fireEvent.click(screen.getByRole("button", { name: "Comparisons", exact: true }));
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Input values: density.json / energy" }));
  renderChart.mockClear();
  fireEvent.click(screen.getByRole("option", { name: "Density per unit x" }));
  expect(renderChart.mock.calls.some(([props]) => props.option.series.some((series) => series.name === "density.json: energy" && JSON.stringify(series.data) === JSON.stringify([[0, 10], [4, 10]])))).toBe(true);
  expect(comparisonBins.map((bin) => bin.value)).toEqual([2, 3]);
});

test("observable search survives polling, blur, result selection and table selection", () => {
  let poll;
  const changes = vi.fn();
  function Harness() {
    const [values, setValues] = useState({});
    const [revision, setRevision] = useState(0);
    poll = () => setRevision((value) => value + 1);
    return <PanelCollection panelSpecs={[{ panel_id: "obs", label: "Observables", kind: "table" }]}
      panelStates={[{ panel_id: "obs", kind: "table", columns: ["Name", "Samples"], rows: [["Energy", revision], ["Angle", revision]], row_keys: ["energy", "angle"],
        payload: { histograms: { energy: { title: "Energy", bins: [] }, angle: { title: "Angle", bins: [] } } } }]}
      panelValues={values} onPanelValueChange={(id, value) => { changes(value.selected_histogram); setValues((previous) => ({ ...previous, [id]: value })); }} />;
  }
  render(<Harness />);
  const search = screen.getByRole("combobox", { name: "Observable" });
  expect(search).toHaveValue("Energy");
  act(() => search.focus());
  fireEvent.change(search, { target: { value: "ang" } });
  expect(changes).not.toHaveBeenCalled();
  act(() => poll());
  expect(search).toHaveValue("ang");
  act(() => search.blur());
  expect(search).toHaveValue("Energy");
  act(() => search.focus());
  expect(search).toHaveValue("ang");
  fireEvent.keyDown(search, { key: "ArrowDown" });
  fireEvent.click(screen.getByRole("option", { name: /Angle/ }));
  expect(changes).toHaveBeenLastCalledWith("angle");
  expect(search).toHaveValue("Angle");
  act(() => search.focus());
  expect(search).toHaveValue("ang");
  act(() => search.blur());
  fireEvent.click(screen.getByRole("button", { name: "All observables (2)" }));
  fireEvent.click(screen.getByRole("cell", { name: "Energy", exact: true }));
  expect(search).toHaveValue("Energy");
  act(() => search.focus());
  expect(search).toHaveValue("ang");
  act(() => poll());
  expect(search).toHaveValue("ang");
});
