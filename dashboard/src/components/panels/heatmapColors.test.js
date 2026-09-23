import { describe, expect, test } from "vitest";
import {
  buildScalarHeatmapScale,
  heatmapColorForValue,
  heatmapLegendGradient,
  readHeatmapColorLimits,
  resetHeatmapColorLimits,
  writeHeatmapColorLimit,
} from "./heatmapColors";

const blue = [29, 78, 216];
const green = [22, 163, 74];
const red = [220, 38, 38];

describe("linear heatmap colors", () => {
  test.each([
    { values: [-11.46, -7.83], min: -11.46, max: -7.83 },
    { values: [2, 8], min: 2, max: 8 },
    { values: [-7.34, 0.49], min: -7.34, max: 0.49 },
    { values: [3, 3], min: 2.7, max: 3.3 },
    { values: [0, 0], min: -1, max: 1 },
    { values: [NaN, Infinity, null], min: 0, max: 1 },
  ])("uses the full data range, padding only degenerate data: $values", ({ values, min, max }) => {
    expect(buildScalarHeatmapScale(values, "linear")).toEqual({ zmin: min, zmax: max });
  });

  test("maps values linearly across Viridis and clips to the selected endpoints", () => {
    const purple = [68, 1, 84];
    const yellow = [253, 231, 37];
    expect(heatmapColorForValue(-11, -11, -8, "linear")).toEqual(purple);
    expect(heatmapColorForValue(-8, -11, -8, "linear")).toEqual(yellow);
    expect(heatmapColorForValue(-20, -11, -8, "linear")).toEqual(purple);
    expect(heatmapColorForValue(20, -11, -8, "linear")).toEqual(yellow);
    for (const fraction of [0.25, 0.5, 0.75]) {
      expect(heatmapColorForValue(-11 + fraction * 3, -11, -8, "linear"))
        .toEqual(heatmapColorForValue(2 + fraction * 6, 2, 8, "linear"));
    }
    const midpoint = heatmapColorForValue(0.5, 0, 1, "linear");
    expect(midpoint[0]).toBeCloseTo(33, 0);
    expect(midpoint[1]).toBeGreaterThanOrEqual(144);
    expect(midpoint[1]).toBeLessThanOrEqual(145);
    expect(midpoint[2]).toBeGreaterThanOrEqual(140);
    expect(midpoint[2]).toBeLessThanOrEqual(141);
    expect(heatmapLegendGradient(-11, -8, "linear")).toMatch(/^linear-gradient\(to top, #440154, .*#fde725\)$/);
  });

  test("supports independent bounds of either sign and ignores invalid or legacy limits", () => {
    expect(buildScalarHeatmapScale([-11, -8], "linear", { max: -9 })).toEqual({ zmin: -11, zmax: -9 });
    expect(buildScalarHeatmapScale([2, 8], "linear", { min: 4 })).toEqual({ zmin: 4, zmax: 8 });
    for (const limits of [{ min: 8, max: 2 }, { min: 4, max: 4 }, { min: NaN, max: Infinity }]) {
      expect(buildScalarHeatmapScale([2, 8], "linear", limits)).toEqual({ zmin: 2, zmax: 8 });
    }
    const legacy = { colorLimits: { metricMode: "log10_integrand", min: -11, max: 0 } };
    expect(readHeatmapColorLimits(legacy, "log10_integrand", "linear")).toBeNull();
    expect(readHeatmapColorLimits(legacy, "log10_integrand", "zero_centered")).toEqual(legacy.colorLimits);
    const next = writeHeatmapColorLimit(legacy, "log10_integrand", "max", -9, "linear");
    expect(readHeatmapColorLimits(next, "log10_integrand", "linear")).toEqual({
      metricMode: "log10_integrand", colorScale: "linear", max: -9,
    });
    expect(readHeatmapColorLimits(next, "log10_integrand", "zero_centered")).toBeNull();
    expect(readHeatmapColorLimits(next, "log10_pdf", "linear")).toBeNull();
  });
});

describe("zero-centered heatmap colors", () => {
  test.each([
    { values: [-7.34, 0.49], min: -7.34, max: 0.49 },
    { values: [-1, 46.94], min: -1, max: 46.94 },
    { values: [-11.46, -7.83], min: -11.46, max: 0 },
    { values: [2, 8], min: 0, max: 8 },
    { values: [3, 3], min: 0, max: 3 },
    { values: [-3, -3], min: -3, max: 0 },
    { values: [0, 0], min: -1, max: 1 },
    { values: [NaN, Infinity, null], min: -1, max: 1 },
  ])("uses data extrema and a zero anchor for $values", ({ values, min, max }) => {
    expect(buildScalarHeatmapScale(values, "zero_centered")).toEqual({ zmin: min, zmax: max });
    expect(heatmapColorForValue(0, min, max, "zero_centered")).toEqual(green);
  });

  test("uses both color endpoints for unequal limits, with zero at the neutral color", () => {
    expect(heatmapColorForValue(-8, -8, 1, "zero_centered")).toEqual(blue);
    expect(heatmapColorForValue(0, -8, 1, "zero_centered")).toEqual(green);
    expect(heatmapColorForValue(1, -8, 1, "zero_centered")).toEqual(red);
    const negativeHalf = heatmapColorForValue(-4, -8, 1, "zero_centered");
    const positiveHalf = heatmapColorForValue(0.5, -8, 1, "zero_centered");
    expect(heatmapColorForValue(-4, -8, 20, "zero_centered")).toEqual(negativeHalf);
    expect(heatmapColorForValue(0.5, -100, 1, "zero_centered")).toEqual(positiveHalf);
    expect(heatmapColorForValue(-20, -8, 1, "zero_centered")).toEqual(blue);
    expect(heatmapColorForValue(20, -8, 1, "zero_centered")).toEqual(red);
  });

  test("uses only the matching half of the palette for one-sided ranges", () => {
    expect(heatmapColorForValue(-8, -8, 0, "zero_centered")).toEqual(blue);
    expect(heatmapColorForValue(8, 0, 8, "zero_centered")).toEqual(red);
    expect(heatmapColorForValue(4, -8, 0, "zero_centered")).toEqual(green);
    expect(heatmapColorForValue(-4, 0, 8, "zero_centered")).toEqual(green);
    expect(heatmapLegendGradient(-8, 0, "zero_centered")).toBe("linear-gradient(to top, #1d4ed8, #16a34a)");
    expect(heatmapLegendGradient(0, 8, "zero_centered")).toBe("linear-gradient(to top, #16a34a, #dc2626)");
    expect(heatmapLegendGradient(-8, 1, "zero_centered")).toContain("#16a34a 50%");
  });

  test("excludes masked pixels and handles large rasters without argument spreading", () => {
    const pixels = new Array(300000).fill(-2);
    pixels[0] = -1000;
    pixels[1] = 1000;
    pixels[2] = NaN;
    expect(buildScalarHeatmapScale(pixels, "zero_centered", null, new Set([0, 1])))
      .toEqual({ zmin: -2, zmax: 0 });
  });

  test("retains symmetric and min/max behavior for other image views", () => {
    expect(buildScalarHeatmapScale([-8, 1], "symmetric")).toEqual({ zmin: -8, zmax: 8 });
    expect(buildScalarHeatmapScale([-8, 1], "min_max")).toEqual({ zmin: -8, zmax: 1 });
    expect(heatmapColorForValue(-3.5, -8, 1, "min_max")).toEqual(green);
  });

  test("keeps limits independent and resets them without losing the view", () => {
    const initial = { zoom: { start: 25, end: 75 }, spread: 3 };
    const minOnly = writeHeatmapColorLimit(initial, "log10_pdf", "min", -4);
    expect(buildScalarHeatmapScale([-8, 1], "zero_centered", minOnly.colorLimits)).toEqual({ zmin: -4, zmax: 1 });
    const both = writeHeatmapColorLimit(minOnly, "log10_pdf", "max", 0.25);
    expect(buildScalarHeatmapScale([-8, 1], "zero_centered", both.colorLimits)).toEqual({ zmin: -4, zmax: 0.25 });
    expect(readHeatmapColorLimits(both, "relative_mismatch")).toBeNull();
    expect(resetHeatmapColorLimits(both)).toEqual({ zoom: initial.zoom });
  });

  test("rejects stale invalid bounds that would exclude zero or collapse the range", () => {
    for (const limits of [{ min: 3, max: -2 }, { min: 0, max: 0 }, { min: NaN, max: Infinity }]) {
      expect(buildScalarHeatmapScale([-8, 1], "zero_centered", limits)).toEqual({ zmin: -8, zmax: 1 });
    }
  });
});
