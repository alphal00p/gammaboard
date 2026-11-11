import { useCallback, useEffect, useRef, useState } from "react";
import { importHwuBundle } from "../../services/api";
import { asArray } from "../../utils/collections";
import { isObject } from "./panelView";
import {
  HISTOGRAM_SORT_CANONICAL,
  histogramSupportsComparison,
  histogramSelectionKey,
  normalizeHistogramSelectionState,
  normalizeHistogramSortMode,
  parseUploadedHistogramBundle,
} from "./histogramUtils";

export const useHistogramBundles = () => {
  const [histogramBundlesByPanel, setHistogramBundlesByPanel] = useState({});
  const [histogramBundleUploadErrors, setHistogramBundleUploadErrors] = useState({});
  const [histogramBundleUploads, setHistogramBundleUploads] = useState({});
  const requests = useRef(new Map());
  useEffect(() => () => { requests.current.forEach((request) => request.abort()); requests.current.clear(); }, []);

  const uploadHistogramBundle = useCallback(async (panelId, event) => {
    const file = event?.target?.files?.[0];
    if (!panelId || !file) return;
    if (requests.current.has(panelId)) return;
    const request = new AbortController();
    requests.current.set(panelId, request);
    setHistogramBundleUploads((current) => ({ ...current, [panelId]: true }));
    try {
      if (file.size > 16 * 1024 * 1024) throw new Error("Histogram bundles must be at most 16 MiB.");
      const text = await file.text();
      if (request.signal.aborted) return;
      const payload = /\.hwu$/i.test(file.name) || text.trimStart().startsWith("<histogram>") || text.trimStart().startsWith("##&")
        ? await importHwuBundle(text, request.signal) : JSON.parse(text);
      if (request.signal.aborted) return;
      const bundle = parseUploadedHistogramBundle(payload);
      setHistogramBundlesByPanel((current) => {
        const existing = asArray(current?.[panelId]);
        const baseLabel = file.name || `bundle-${existing.length + 1}`;
        let label = baseLabel;
        let suffix = 2;
        while (existing.some((entry) => entry.label === label)) label = `${baseLabel} (${suffix++})`;
        return {
          ...current,
          [panelId]: [
            ...existing,
            {
              id: `overlay-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
              label,
              histograms: bundle.histograms,
              warnings: asArray(payload.warnings).filter((warning) => typeof warning === "string"),
              selectionsByHistogram: {},
            },
          ],
        };
      });
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: null }));
    } catch (error) {
      if (request.signal.aborted) return;
      const message = error instanceof Error ? error.message : "Invalid histogram bundle.";
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: message }));
    } finally {
      requests.current.delete(panelId);
      if (!request.signal.aborted) setHistogramBundleUploads((current) => ({ ...current, [panelId]: false }));
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

  const removeComparedHistogram = useCallback((panelId, bundleId, histogramName, comparedIndex, histogramTitle, isDiscrete) => {
    if (!panelId || !bundleId || !Number.isInteger(comparedIndex)) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName, { histogramTitle, isDiscrete });
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

  const addComparedHistogram = useCallback((panelId, bundleId, histogramName, currentHistogramIsDiscrete, histogramTitle) => {
    if (!panelId || !bundleId) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName, { histogramTitle, isDiscrete: currentHistogramIsDiscrete });
        const histogramNames = Object.keys(bundle?.histograms || {}).filter(
          (name) => histogramSupportsComparison(bundle?.histograms?.[name], Boolean(currentHistogramIsDiscrete)),
        );
        const used = new Set(asArray(selection.selectedHistograms).filter((value) => typeof value === "string"));
        const available = histogramNames.filter((name) => !used.has(name));
        // Match the visible table title to the dropdown label (the bundle key).
        const candidate = available.includes(histogramTitle) ? histogramTitle : available[0];
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
                [candidate]: bundle.histograms[candidate]?.source_format === "hwu" ? "by_index" : "by_key",
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
    histogramBundleUploads,
    uploadHistogramBundle,
    removeHistogramBundle,
    updateHistogramBundleSelection,
    removeComparedHistogram,
    addComparedHistogram,
  };
};
