import { expect, test } from "vitest";
import {
  comparisonModels,
  mismatchColor,
  MATCH_COLOR,
  mismatchValue,
  planeOption,
  plotColor,
} from "./pdfDisplay";
import { diagnosticExportData } from "./pdfDiagnosticExport";
const state = {
  kind: "pdf_comparison",
  geometry: {
    kind: "plane",
    geometry: {
      u_linspace: { start: 0, stop: 1, count: 2 },
      v_linspace: { start: 0, stop: 1, count: 2 },
    },
  },
  data: {
    integrand_values: [1, 0, 0, -1],
    pdf_values: [0, 1, 0, 2],
    reference: { value: 1, error: 0.1 },
  },
};
const settings = {
  normalization: "reference",
  metric: "log_ratio",
  palette: "Viridis",
  auxiliary: "mismatch",
  log: false,
};
test("asymmetric mismatch limits pin green to zero and retain infinite/undefined cases", () => {
  expect(mismatchColor(0, { min: -1, max: 10 })).toBe(MATCH_COLOR);
  expect(mismatchColor(-1, { min: -1, max: 10 })).toBe("#c82d3c");
  expect(mismatchColor(10, { min: -1, max: 10 })).toBe("#2466c5");
  const models = comparisonModels(state, "overall", settings);
  expect(mismatchValue(models.c, 0, "log_ratio")).toBe(-Infinity);
  expect(mismatchValue(models.c, 1, "log_ratio")).toBe(Infinity);
  expect(mismatchValue(models.c, 2, "relative")).toBeNull();
  expect(plotColor(models.mismatch, 0)).toBe("#c82d3c");
  expect(plotColor(models.mismatch, 1)).toBe("#2466c5");
  expect(plotColor(models.mismatch, 2)).toBe("#e5e5e5");
  expect(models.target.limits).toEqual(models.sampler.limits);
  expect(planeOption(models.sign).series[0].data[3].itemStyle.color).toBe(
    "#d55e00",
  );
});
test("exports preserve raw data, selected extent, reference uncertainty and exceptional values", () => {
  const full = diagnosticExportData(
    state,
    settings,
    "overall",
    "mismatch",
    null,
  );
  expect(full.reference.error).toBe(0.1);
  expect(full.points.map((p) => p.raw_projection)).toEqual([1, 0, 0, -1]);
  expect(full.points[0].log_ratio).toBe("-Infinity");
  expect(full.points[1].log_ratio).toBe("+Infinity");
  const zoom = { x: { start: 50, end: 100 }, y: { start: 0, end: 100 } };
  const cut = diagnosticExportData(state, settings, "overall", "target", zoom);
  expect(cut.points).toHaveLength(2);
  const hist = diagnosticExportData(
    state,
    settings,
    "overall",
    "densityHistogram",
    zoom,
  );
  expect(hist.points).toHaveLength(4);
  expect(hist.histograms).toHaveLength(2);
});
