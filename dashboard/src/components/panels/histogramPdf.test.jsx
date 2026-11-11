import { act, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { renderHistogramBundlePdf } from "./histogramPdf";

const { options, convert } = vi.hoisted(() => ({ options: [], convert: vi.fn(async () => {}) }));
vi.mock("svg2pdf.js", () => ({ svg2pdf: convert }));
vi.mock("./pdfFont", () => ({ registerPdfFont: async () => {} }));
vi.mock("../../lib/echarts", () => ({ echarts: { init: () => ({
  setOption: (option) => options.push(option), renderToSVGString: () => '<svg xmlns="http://www.w3.org/2000/svg"/>', dispose: vi.fn(),
}) } }));
beforeEach(() => { options.length = 0; convert.mockClear(); });

const histogram = { title: "Energy", bins: [
  { start: 0, stop: 1, value: -1, error: 0.3 }, { start: 1, stop: 2, value: 4, error: 0.4 },
] };
const payload = { histograms: { hidden_key: histogram, angle: { ...histogram, title: "Angle" } } };
const bundles = [{ label: "reference.json", id: "ref", histograms: {
  fallback: { ...histogram, title: "Other" }, Energy: histogram, Angle: histogram,
}, selectionsByHistogram: { angle: { selectedHistograms: ["fallback"] } } }];

test.each([false, true])("exports every observable on one page with display rebinning (comparisons: %s)", async (includeComparisons) => {
  let result;
  let promise;
  await act(async () => {
    promise = renderHistogramBundlePdf({ payload, panelId: "campaign_histograms", includeComparisons,
      uploadedBundles: bundles, value: { histogram_view: { show_relative_error: false,
        rebin_factors: { '["hidden_key",null,null]': 2 } } },
    }).then((blob) => { result = blob; });
  });
  await waitFor(() => expect(result).toBeDefined());
  await promise;
  expect(convert).toHaveBeenCalledTimes(includeComparisons ? 4 : 2);
  expect(options[0].series.find((series) => series.name === "value").data).toEqual([[0, 3], [2, 3]]);
  const names = options.filter((option) => option.series.some((series) => series.name === "value")).map((option) => option.series.map((series) => series.name));
  expect(names[0].includes("reference.json: Energy")).toBe(includeComparisons);
  expect(names[1].includes("reference.json: fallback")).toBe(includeComparisons);
  const pdf = await new Promise((resolve) => {
    const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.readAsText(result);
  });
  expect(pdf).toMatch(/^%PDF-/);
  expect(pdf.match(/\/Type \/Page\b/g)).toHaveLength(2);
  expect(histogram.bins.map((bin) => bin.value)).toEqual([-1, 4]);
});
