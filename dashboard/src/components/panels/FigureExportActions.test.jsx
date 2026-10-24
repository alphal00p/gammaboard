import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import FigureExportActions from "./FigureExportActions";

afterEach(() => vi.restoreAllMocks());

test("canvas-rendered ECharts exports SVG paths rather than an embedded PNG", async () => {
  let blob;
  URL.createObjectURL = vi.fn((value) => { blob = value; return "blob:figure"; });
  URL.revokeObjectURL = vi.fn();
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  const instance = { getWidth: () => 400, getHeight: () => 300, getOption: () => ({
    xAxis: { type: "value" }, yAxis: { type: "value" },
    series: [{ type: "line", data: [[0, 1], [1, 2]] }],
  }) };
  render(<FigureExportActions baseName="line" payload={{}} echartsRef={{ current: { getEchartsInstance: () => instance } }} />);
  fireEvent.click(screen.getByRole("button", { name: "SVG" }));
  await waitFor(() => expect(blob).toBeDefined());
  const text = await new Promise((resolve) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.readAsText(blob); });
  expect(text).toContain("<path");
  expect(text).not.toContain("data:image/png");
});

test("raster images download with an honest PNG extension", () => {
  const canvas = document.createElement("canvas");
  const figure = document.createElement("div"); figure.append(canvas);
  vi.spyOn(canvas, "toDataURL").mockReturnValue("data:image/png;base64,aA==");
  let name;
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function () { name = this.download; });
  render(<FigureExportActions baseName="heatmap" payload={{}} imageFormat="png" elementRef={{ current: figure }} />);
  fireEvent.click(screen.getByRole("button", { name: "PNG" }));
  expect(name).toBe("heatmap.png");
});


test("CSV keeps the chosen estimator and never substitutes basic error for a missing primary error", async () => {
  const { figureCsv } = await import("./FigureExportActions");
  const csv = figureCsv({ state: { entries: [{ label: "x,y", value: { kind: "estimate", value: 1.5, error: null, estimator: "stratified", basic: { mean: 1.8, error: .7 }, sample_count: 4 } }] } });
  expect(csv).toContain("estimator,value,primary_error,strat_err,basic_mean,basic_err,uncertainty_status,samples");
  expect(csv).toContain('"x,y",,stratified,1.5,,,1.8,0.7,unavailable,4');
  const zero = figureCsv({ state: { entries: [{ label: "constant", value: { kind: "estimate", value: 0, error: 0, estimator: "pooled", sample_count: 2 } }] } });
  expect(zero).toContain("constant,,pooled,0,0,,0,0,estimated_from_samples,2");
});


test("history CSV preserves tiny explicit errors even when band endpoints round together", async () => {
  const { figureCsv } = await import("./FigureExportActions");
  const csv = figureCsv({ state: { x_axis: "completed_samples", points: [{ x: 32, y: 1e20, error: .5, y_min: 1e20, y_max: 1e20, estimator: "stratified", basic: { mean: 1e20, error: 1 } }] } });
  expect(csv).toContain("stratified,100000000000000000000,0.5,0.5,100000000000000000000,1,estimated_from_samples,32");
});
