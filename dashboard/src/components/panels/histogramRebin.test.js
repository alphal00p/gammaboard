import { describe, expect, test } from "vitest";
import { rebinHistogramBins, alignComparisonBins, alignCumulativeBins } from "./histogramRebin";
import { parseUploadedHistogramBundle, buildCdfBins, writeHistogramBundlePanelValue, histogramSupportsComparison, normalizeHistogramSelectionState } from "./histogramUtils";

test("imports derived campaign moments as the aggregate without mixing optional child observables", () => {
  const bundle = parseUploadedHistogramBundle({ statistics_kind: "derived_moments", histograms: {
    energy: { title: "Energy", sample_count: 4, bins: [
      { x_min: 0, x_max: 1, sum_weights: 20, sum_weights_squared: 124, entry_count: 4 },
    ] },
  }, children: [{ bundle: { histograms: { unrelated: { bins: [] } } } }] });
  expect(Object.keys(bundle.histograms)).toEqual(["energy"]);
  expect(bundle.histograms.energy.bins[0].value).toBe(5);
  expect(bundle.histograms.energy.bins[0].error).toBeCloseTo(Math.sqrt(2), 12);
});

const bins = [
  { start: 0, stop: 1, value: 3, error: 0.3, sum_weights_squared: 99 },
  { start: 1, stop: 3, value: -2, error: 0.4 },
  { start: 3, stop: 4, value: 5, error: null },
];

describe("display rebinning", () => {
  test("merges signed integrals and errors, preserving a partial final group and original data", () => {
    const original = structuredClone(bins);
    const merged = rebinHistogramBins(bins, 2);
    expect(merged).toEqual([
      { start: 0, stop: 3, x: 1.5, value: 1, error: 0.5 },
      { start: 3, stop: 4, x: 3.5, value: 5, error: null },
    ]);
    expect(bins).toEqual(original);
    expect(rebinHistogramBins(bins, 1)).toEqual(original);
    expect(merged[0]).not.toHaveProperty("sum_weights_squared");
    expect(buildCdfBins(merged).at(-1)).toMatchObject({ value: 6, error: null });
  });
  test("converts differential density to integrated bins before merging unequal widths", () => {
    expect(rebinHistogramBins(bins.slice(0, 2), 2, true)[0]).toMatchObject({ value: -1, error: Math.hypot(0.3, 0.8) });
  });
  test("rejects invalid factors, categorical bins, overlaps and gaps without changing input", () => {
    for (const factor of [0, -1, 1.5, 4, NaN]) expect(() => rebinHistogramBins(bins, factor)).toThrow();
    expect(() => rebinHistogramBins(bins.map((bin) => ({ ...bin, label: "category" })), 2)).toThrow(/continuous/);
    expect(() => rebinHistogramBins([bins[0], bins[2]], 2)).toThrow(/gap/);
    expect(() => rebinHistogramBins([bins[0], { ...bins[1], start: 0.5 }], 2)).toThrow(/overlap/);
  });
  test("aligns only exact coarsenings and never splits a wider bin", () => {
    const merged = rebinHistogramBins(bins, 2);
    expect(alignComparisonBins(merged, bins)).toEqual(merged);
    expect(alignComparisonBins(bins, merged).slice(0, 2)).toEqual([null, null]);
    expect(alignComparisonBins([{ start: 0, stop: 4 }], [bins[0], bins[2]])).toEqual([null]);
    expect(() => rebinHistogramBins([{ start: 0, stop: 1e-12, value: 1 }, { start: 2e-12, stop: 3e-12, value: 2 }], 2)).toThrow(/gap/);
  });
  test("cumulative ratios compare equal cumulative ranges without summing CDF values", () => {
    const cumulative = buildCdfBins(bins);
    const merged = buildCdfBins(rebinHistogramBins(bins, 2));
    expect(alignCumulativeBins(merged, cumulative).map((bin) => bin.value)).toEqual([1, 6]);
    expect(alignCumulativeBins(cumulative, merged).map((bin) => bin?.value ?? null)).toEqual([null, 1, 6]);
    expect(alignCumulativeBins(merged, cumulative.slice(1))).toEqual([null, null]);
  });
  test("accepts unavailable JSON errors and preserves imported metadata", () => {
    const imported = parseUploadedHistogramBundle({ histograms: { energy: { title: "Energy", type_description: "LO", bins } } });
    expect(imported.histograms.energy.bins[2].error).toBeNull();
    expect(imported.histograms.energy.type_description).toBe("LO");
    expect(() => parseUploadedHistogramBundle({ histograms: { energy: { bins: [{ start: 0, stop: 1, value: 2, error: "invalid" }] } } })).toThrow(/uncertainty/);
  });
  test("HwU unit integer bins allow explicit discrete comparison without inventing labels", () => {
    const histogram = { source_format: "hwu", bins: [{ start: 0, stop: 1, value: 2, error: 0.1 }] };
    expect(histogramSupportsComparison(histogram, true)).toBe(true);
    expect(histogramSupportsComparison(histogram, false)).toBe(true);
    expect(histogramSupportsComparison({ ...histogram, bins: [{ start: 0, stop: 2 }] }, true)).toBe(false);
    expect(normalizeHistogramSelectionState({ histograms: { integral: histogram } }, "integral").discreteAlignmentByHistogram.integral).toBe("by_index");
  });
  test("view factors survive other controls and reset independently of density conventions", () => {
    const value = writeHistogramBundlePanelValue({}, { rebinFactors: { energy: 2 }, densityInputs: { reference: true } });
    const updated = writeHistogramBundlePanelValue(value, { xScale: "log" });
    expect(updated.histogram_view.rebin_factors.energy).toBe(2);
    const reset = writeHistogramBundlePanelValue(updated, { resetRebin: true });
    expect(reset.histogram_view.rebin_factors).toBeUndefined();
    expect(reset.histogram_view.density_inputs.reference).toBe(true);
  });
});
