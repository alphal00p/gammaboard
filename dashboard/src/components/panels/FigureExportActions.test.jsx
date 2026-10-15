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
