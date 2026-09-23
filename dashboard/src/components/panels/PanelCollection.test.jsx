import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
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

describe("PDF adaptation heatmap controls", () => {
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

  test("shares zoom while preserving each plot's color limits", () => {
    const values = Object.fromEntries(pdfImageSpecs.map((spec, index) => [spec.panel_id, {
      zoom: { start: 25, end: 75 },
      colorLimits: { metricMode: "log10_ratio", min: -index - 2, max: index + 2 },
    }]));
    const changed = vi.fn();
    render(<PanelCollection
      panelSpecs={pdfImageSpecs}
      panelStates={pdfImageSpecs.map((spec) => pdfImageState(spec))}
      panelValues={values}
      onPanelValueChange={changed}
    />);
    const card = screen.getByText("log_integrand").closest(".MuiCard-root");
    fireEvent.click(within(card).getByRole("button", { name: "Reset" }));
    expect(changed).toHaveBeenCalledTimes(3);
    for (const [id, value] of changed.mock.calls) {
      expect(value.zoom).toEqual({ start: 0, end: 100 });
      expect(value.colorLimits).toEqual(values[id].colorLimits);
    }
  });
});

const renderTimeseries = ({ kind = "scalar_timeseries", panelId = "test_history", xAxis, points, value, target }) => {
  const state = kind === "scalar_timeseries"
    ? { points, target }
    : { series: [{ id: "test", label: "Test", points }] };
  render(
    <PanelCollection
      panelSpecs={[{ panel_id: panelId, label: "Test", kind, history: "append" }]}
      panelStates={[{ ...state, panel_id: panelId, x_axis: xAxis }]}
      panelValues={{ [panelId]: value }}
    />,
  );
  return renderChart.mock.lastCall[0].option;
};

const tooltipHeader = (option, x) => option.tooltip.formatter([
  { axisValue: x, seriesName: "Test", value: [x, 0.8] },
]).split("<br/>")[0];

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
