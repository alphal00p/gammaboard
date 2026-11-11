import { expect, test } from "vitest";
import { normalizeHistogramSelectionState } from "./histogramUtils";

const continuous = { bins: [{ start: 0, stop: 1, value: 2 }] };
test("default comparisons require a compatible identity or unambiguous title", () => {
  const select = (histograms, context) => normalizeHistogramSelectionState({ histograms }, "energy", { histogramTitle: "Energy", isDiscrete: false, ...context }).selectedHistograms;
  expect(select({ unrelated: continuous })).toEqual([]);
  expect(select({ energy: continuous })).toEqual(["energy"]);
  expect(select({ Energy: continuous, energy: continuous })).toEqual(["Energy"]);
  expect(select({ first: { ...continuous, title: "Energy" } })).toEqual(["first"]);
  expect(select({ first: { ...continuous, title: "Energy" }, second: { ...continuous, title: "Energy" } })).toEqual([]);
  expect(select({ energy: { bins: [{ bin_id: 1, label: "one", value: 2 }] } })).toEqual([]);
  expect(normalizeHistogramSelectionState({ histograms: { energy: continuous }, selectionsByHistogram: { energy: { selectedHistograms: [] } } }, "energy").selectedHistograms).toEqual([]);
});
