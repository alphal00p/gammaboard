import { useCallback, useState } from "react";
import { asArray } from "../../utils/collections";
import { isObject } from "./panelView";
import {
  HISTOGRAM_SORT_CANONICAL,
  histogramIsDiscrete,
  histogramSelectionKey,
  normalizeHistogramSelectionState,
  normalizeHistogramSortMode,
  parseUploadedHistogramBundle,
} from "./histogramUtils";

export const useHistogramBundles = () => {
  const [histogramBundlesByPanel, setHistogramBundlesByPanel] = useState({});
  const [histogramBundleUploadErrors, setHistogramBundleUploadErrors] = useState({});

  const uploadHistogramBundle = useCallback(async (panelId, event) => {
    const file = event?.target?.files?.[0];
    if (!panelId || !file) return;
    try {
      const text = await file.text();
      const bundle = parseUploadedHistogramBundle(JSON.parse(text));
      setHistogramBundlesByPanel((current) => {
        const existing = asArray(current?.[panelId]);
        return {
          ...current,
          [panelId]: [
            ...existing,
            {
              id: `overlay-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
              label: file.name || `bundle-${existing.length + 1}`,
              histograms: bundle.histograms,
              selectionsByHistogram: {},
            },
          ],
        };
      });
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: null }));
    } catch (error) {
      const message = error instanceof Error ? error.message : "Invalid histogram bundle JSON.";
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: message }));
    } finally {
      if (event?.target) event.target.value = "";
    }
  }, []);

  const removeHistogramBundle = useCallback((panelId, bundleId) => {
    if (!panelId || !bundleId) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).filter((bundle) => bundle?.id !== bundleId),
    }));
  }, []);

  const updateHistogramBundleSelection = useCallback(
    (
      panelId,
      bundleId,
      histogramName,
      selectedHistograms,
      discreteAlignmentByHistogram = {},
      sortModeByHistogram = {},
    ) => {
      if (!panelId || !bundleId) return;
      const key = histogramSelectionKey(histogramName);
      setHistogramBundlesByPanel((current) => ({
        ...current,
        [panelId]: asArray(current?.[panelId]).map((bundle) => {
          if (bundle?.id !== bundleId) return bundle;
          return {
            ...bundle,
            selectionsByHistogram: {
              ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
              [key]: {
                selectedHistograms: asArray(selectedHistograms)
                  .filter((value) => typeof value === "string")
                  .filter((value, index, values) => values.indexOf(value) === index),
                discreteAlignmentByHistogram: Object.fromEntries(
                  Object.entries(isObject(discreteAlignmentByHistogram) ? discreteAlignmentByHistogram : {})
                    .filter(([name, value]) => typeof name === "string" && typeof value === "string")
                    .map(([name, value]) => [name, value === "by_index" ? "by_index" : "by_key"]),
                ),
                sortModeByHistogram: Object.fromEntries(
                  Object.entries(isObject(sortModeByHistogram) ? sortModeByHistogram : {})
                    .filter(([name]) => typeof name === "string")
                    .map(([name, value]) => [name, normalizeHistogramSortMode(value)]),
                ),
              },
            },
          };
        }),
      }));
    },
    [],
  );

  const removeComparedHistogram = useCallback((panelId, bundleId, histogramName, comparedIndex) => {
    if (!panelId || !bundleId || !Number.isInteger(comparedIndex)) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName);
        const nextSelectedHistograms = selection.selectedHistograms.filter((_, index) => index !== comparedIndex);
        return {
          ...bundle,
          selectionsByHistogram: {
            ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
            [selection.key]: {
              selectedHistograms: nextSelectedHistograms,
              discreteAlignmentByHistogram: Object.fromEntries(
                Object.entries(selection.discreteAlignmentByHistogram || {}).filter(([name]) =>
                  nextSelectedHistograms.includes(name),
                ),
              ),
              sortModeByHistogram: Object.fromEntries(
                Object.entries(selection.sortModeByHistogram || {}).filter(([name]) =>
                  nextSelectedHistograms.includes(name),
                ),
              ),
            },
          },
        };
      }),
    }));
  }, []);

  const addComparedHistogram = useCallback((panelId, bundleId, histogramName, currentHistogramIsDiscrete) => {
    if (!panelId || !bundleId) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName);
        const histogramNames = Object.keys(bundle?.histograms || {}).filter(
          (name) => histogramIsDiscrete(bundle?.histograms?.[name]) === Boolean(currentHistogramIsDiscrete),
        );
        const used = new Set(asArray(selection.selectedHistograms).filter((value) => typeof value === "string"));
        const preferred =
          typeof histogramName === "string" && histogramNames.includes(histogramName) ? histogramName : null;
        const candidate =
          preferred && !used.has(preferred) ? preferred : histogramNames.find((name) => !used.has(name)) || null;
        if (typeof candidate !== "string") return bundle;
        return {
          ...bundle,
          selectionsByHistogram: {
            ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
            [selection.key]: {
              selectedHistograms: [...selection.selectedHistograms, candidate].filter(
                (value, index, values) => values.indexOf(value) === index,
              ),
              discreteAlignmentByHistogram: {
                ...(selection.discreteAlignmentByHistogram || {}),
                [candidate]: "by_key",
              },
              sortModeByHistogram: {
                ...(selection.sortModeByHistogram || {}),
                [candidate]: HISTOGRAM_SORT_CANONICAL,
              },
            },
          },
        };
      }),
    }));
  }, []);

  return {
    histogramBundlesByPanel,
    histogramBundleUploadErrors,
    uploadHistogramBundle,
    removeHistogramBundle,
    updateHistogramBundleSelection,
    removeComparedHistogram,
    addComparedHistogram,
  };
};
