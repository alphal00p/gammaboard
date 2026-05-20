import { Suspense, forwardRef, lazy, useMemo, useRef, useState } from "react";
import { Box, Button, Card, CardContent, FormControl, MenuItem, Select, Stack, Typography } from "@mui/material";
import { asArray } from "../../utils/collections";
import { formatScientific } from "../../utils/formatters";
import FigureExportActions from "./FigureExportActions";
import {
  HISTOGRAM_MODE_CDF,
  HISTOGRAM_MODE_PDF,
  HISTOGRAM_SORT_BY_ABS_VALUE,
  HISTOGRAM_SORT_BY_VALUE,
  HISTOGRAM_SORT_CANONICAL,
  buildCdfBins,
  buildDiscreteRelativeErrorData,
  buildHistogramData,
  buildHistogramRenderData,
  buildHistogramYDomain,
  buildRelativeErrorStepData,
  buildRelativeErrorYDomain,
  discreteHistogramBinKey,
  fitDomain,
  fitHistogramXDomain,
  histogramIsDiscrete,
  normalizeHistogramMode,
  normalizeHistogramSelectionState,
  normalizeHistogramSortMode,
  projectOverlayHistogramToReferenceBins,
  readHistogramBundleView,
  readHistogramViewIdFromPanelValue,
  readHistogramScaleFromPanelValue,
  readHistogramYZoomFromPanelValue,
  readHistogramZoomFromPanelValue,
  signedLog10,
  sortHistogramBinsByMode,
  writeHistogramBundlePanelValue,
} from "./histogramUtils";
import {
  histogramControlDefault,
  histogramControlEnabled,
  metricLabel,
  metricNumber,
  normalizeHistogramViews,
  projectBinsForHistogramView,
  resolveHistogramView,
} from "./histogramViews";
import {
  FULL_ZOOM,
  isObject,
  readDataZoomRanges,
  readYZoomFromPanelValue,
  readZoomFromPanelValue,
  visibleXRangeFromZoomWithScale,
  writeZoomPanelValue,
  zoomRangeChanged,
} from "./panelView";
import { buildHistogramOption, buildRatioHistogramOption, buildRelativeHistogramOption } from "./histogramOptions";

const ReactECharts = lazy(() =>
  Promise.all([import("echarts-for-react"), import("../../lib/echarts")]).then(([module]) => ({
    default: module.default,
  })),
);

const LazyChart = forwardRef((props, ref) => (
  <Suspense
    fallback={
      <Box
        sx={{
          width: "100%",
          height: "100%",
          minHeight: 160,
          display: "grid",
          placeItems: "center",
          color: "text.secondary",
          typography: "body2",
        }}
      >
        Loading chart...
      </Box>
    }
  >
    <ReactECharts ref={ref} {...props} />
  </Suspense>
));
LazyChart.displayName = "LazyChart";

const histogramOverlayColors = ["#9b2226", "#3a86ff", "#ff006e", "#6a994e", "#ff7f11", "#8338ec"];

const inferDefaultHistogramYScale = (_state) => "linear";

const inferDefaultHistogramXScale = (state) => (state?.log_x_axis ? "log" : "linear");

const HistogramPanel = ({
  title,
  state,
  value = undefined,
  onValueChange = null,
  uploadedBundles = [],
  onUpdateBundleSelection = null,
  onRemoveComparedHistogram = null,
  onAddComparedHistogram = null,
}) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const panelId = state?.panel_id || null;
  const sourcePanelId = state?.source_panel_id || panelId;
  const isBundleControlled = Boolean(sourcePanelId && sourcePanelId !== panelId);
  const currentHistogramName = typeof state?.name === "string" ? state.name : null;
  const defaultYScale = inferDefaultHistogramYScale(state);
  const defaultXScale = inferDefaultHistogramXScale(state);
  const declaredViews = useMemo(() => normalizeHistogramViews(state?.views), [state?.views]);
  const [localSelectedViewId, setLocalSelectedViewId] = useState(null);
  const selectedViewId = isBundleControlled
    ? readHistogramViewIdFromPanelValue(value, currentHistogramName)
    : localSelectedViewId;
  const selectedView = useMemo(
    () => resolveHistogramView(declaredViews, selectedViewId),
    [declaredViews, selectedViewId],
  );
  const hasDeclaredViews = declaredViews.length > 0;
  const controls = isObject(state?.controls) ? state.controls : null;
  const metricDescriptors = isObject(state?.metric_descriptors) ? state.metric_descriptors : null;
  const [localYScale, setLocalYScale] = useState("linear");
  const [localXScale, setLocalXScale] = useState("linear");
  const [localSortMode, setLocalSortMode] = useState(HISTOGRAM_SORT_CANONICAL);
  const [localShowRelativeErrors, setLocalShowRelativeErrors] = useState(() =>
    Boolean(histogramControlDefault(controls, "default_relative_error", true)),
  );
  const [localHistogramMode, setLocalHistogramMode] = useState(HISTOGRAM_MODE_PDF);
  const [localShowRatio, setLocalShowRatio] = useState(false);
  const yScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "y", defaultYScale) : localYScale;
  const xScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "x", defaultXScale) : localXScale;
  const zoomRange = isBundleControlled
    ? readHistogramZoomFromPanelValue(value, FULL_ZOOM)
    : readZoomFromPanelValue(value, FULL_ZOOM);
  const yZoomRange = isBundleControlled
    ? readHistogramYZoomFromPanelValue(value, FULL_ZOOM)
    : readYZoomFromPanelValue(value, FULL_ZOOM);
  const view = isBundleControlled ? readHistogramBundleView(value) : {};
  const showRelativeErrors = isBundleControlled ? view.show_relative_error !== false : localShowRelativeErrors;
  const histogramMode = isBundleControlled ? normalizeHistogramMode(view.display_mode) : localHistogramMode;
  const showRatio = isBundleControlled ? view.show_ratio === true : localShowRatio;
  const showScaleControls = histogramControlEnabled(controls, "scale", true);
  const showXScaleControl = histogramControlEnabled(controls, "x_scale", true);
  const showPdfCdfControl = histogramControlEnabled(controls, "pdf_cdf", true);
  const showRatioControl = histogramControlEnabled(controls, "ratio", true);
  const showRelativeErrorControl = histogramControlEnabled(controls, "relative_error", true);
  const showSortControl = histogramControlEnabled(controls, "sort", true);
  const showExportAction = histogramControlEnabled(controls, "export", true);
  const showResetViewAction = histogramControlEnabled(controls, "reset_view", true);
  const requestedSortMode = isBundleControlled
    ? normalizeHistogramSortMode(
        isObject(view?.sort_mode_by_histogram) && currentHistogramName
          ? view.sort_mode_by_histogram[currentHistogramName]
          : HISTOGRAM_SORT_CANONICAL,
      )
    : normalizeHistogramSortMode(localSortMode);
  const baseCanonicalBins = useMemo(() => {
    const rawBins = buildHistogramData(state?.bins);
    if (!hasDeclaredViews) return rawBins;
    return projectBinsForHistogramView(rawBins, selectedView);
  }, [hasDeclaredViews, selectedView, state?.bins]);
  const sortedBaseBins = useMemo(() => {
    const isDiscreteHistogram = baseCanonicalBins.some((bin) => bin && (bin.label != null || bin.bin_id != null));
    return isDiscreteHistogram
      ? sortHistogramBinsByMode(baseCanonicalBins, requestedSortMode)
      : baseCanonicalBins.slice().sort((left, right) => Number(left?.start) - Number(right?.start));
  }, [baseCanonicalBins, requestedSortMode]);
  const bins = useMemo(
    () => (histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(sortedBaseBins) : sortedBaseBins),
    [histogramMode, sortedBaseBins],
  );
  const stepData = useMemo(() => buildHistogramRenderData(bins, yScale), [bins, yScale]);
  const relativeErrorData = useMemo(() => buildRelativeErrorStepData(bins), [bins]);
  // Detect discrete histograms: presence of bin labels/bin_id or explicit discrete ordering
  const isDiscrete = useMemo(() => {
    if (!Array.isArray(state?.bins)) return false;
    if (state?.discrete_ordering) return true;
    return state.bins.some((b) => b && (b.label != null || b.bin_id != null));
  }, [state]);

  const categories = useMemo(() => {
    if (!isDiscrete) return null;
    return bins.map((bin, idx) => {
      if (bin?.label != null) return String(bin.label);
      if (bin?.bin_id != null) return String(bin.bin_id);
      // fallback to range if available
      const start = Number(bin?.start);
      const stop = Number(bin?.stop);
      if (Number.isFinite(start) && Number.isFinite(stop))
        return `${formatScientific(start, 4)}→${formatScientific(stop, 4)}`;
      return `#${idx}`;
    });
  }, [isDiscrete, bins]);
  const discreteRelativeErrorData = useMemo(() => buildDiscreteRelativeErrorData(bins), [bins]);
  const pdfComparisonData = useMemo(() => {
    if (!isDiscrete || histogramMode === HISTOGRAM_MODE_CDF || selectedView.kind !== "bar_with_marker") return null;
    const markerLabel = metricLabel(metricDescriptors, selectedView.marker_metric, "marker");
    const deltaLabel = metricLabel(metricDescriptors, selectedView.delta_metric, "delta");
    const markerData = bins.map((bin) => {
      const raw = metricNumber(bin, selectedView.marker_metric);
      if (!Number.isFinite(raw)) return null;
      return {
        value: yScale === "log" ? signedLog10(raw) : raw,
        rawValue: raw,
      };
    });
    const deltaData = bins
      .map((bin, index) => {
        const rawValue = Number(bin?.value);
        const rawMarker = metricNumber(bin, selectedView.marker_metric);
        if (!Number.isFinite(rawValue) || !Number.isFinite(rawMarker)) return null;
        const value = yScale === "log" ? signedLog10(rawValue) : rawValue;
        const marker = yScale === "log" ? signedLog10(rawMarker) : rawMarker;
        if (!Number.isFinite(value) || !Number.isFinite(marker)) return null;
        return [index, Math.min(value, marker), Math.max(value, marker)];
      })
      .filter(Boolean);
    if (!markerData.some((entry) => entry != null)) return null;
    return { markerData, deltaData, markerLabel, deltaLabel };
  }, [bins, histogramMode, isDiscrete, metricDescriptors, selectedView, yScale]);
  const hasPositiveContinuousEdges = useMemo(
    () =>
      asArray(bins).some((bin) => {
        const start = Number(bin?.start);
        const stop = Number(bin?.stop);
        return Number.isFinite(start) && Number.isFinite(stop) && (start > 0 || stop > 0);
      }),
    [bins],
  );
  const effectiveXScale =
    isDiscrete || xScale !== "log" || hasPositiveContinuousEdges ? (isDiscrete ? "linear" : xScale) : "linear";
  const discreteBaseKeys = useMemo(() => bins.map((bin, index) => discreteHistogramBinKey(bin, index)), [bins]);
  const discreteBaseSortedCanonicalIndices = useMemo(() => {
    if (!isDiscrete) return [];
    const canonicalIndexByBin = new Map(asArray(baseCanonicalBins).map((bin, index) => [bin, index]));
    return asArray(sortedBaseBins).map((bin) => {
      const index = canonicalIndexByBin.get(bin);
      return Number.isFinite(index) ? index : -1;
    });
  }, [baseCanonicalBins, sortedBaseBins, isDiscrete]);
  const comparedBundleSelections = useMemo(
    () =>
      asArray(uploadedBundles).map((bundle) => ({
        bundle,
        selectionState: normalizeHistogramSelectionState(bundle, currentHistogramName),
      })),
    [currentHistogramName, uploadedBundles],
  );
  const comparedOverlaySeries = useMemo(
    () =>
      comparedBundleSelections
        .flatMap(({ bundle, selectionState }, bundleIndex) => {
          const selectedNames = asArray(selectionState.selectedHistograms)
            .filter((name) => typeof name === "string")
            .filter((name, index, values) => values.indexOf(name) === index);
          return selectedNames.map((selectedName, selectedIndex) => {
            const histogram = bundle.histograms?.[selectedName];
            if (!histogram) return null;
            if (histogramIsDiscrete(histogram) !== isDiscrete) return null;
            const discreteMatchMode =
              selectionState.discreteAlignmentByHistogram?.[selectedName] === "by_index" ? "by_index" : "by_key";
            const color = histogramOverlayColors[(bundleIndex * 3 + selectedIndex) % histogramOverlayColors.length];
            const seriesLabel = `${bundle.label}: ${selectedName}`;
            if (isDiscrete) {
              const overlayCanonicalBins = buildHistogramData(histogram.bins);
              const overlayBinByKey = new Map(
                overlayCanonicalBins.map((bin, index) => [discreteHistogramBinKey(bin, index), bin]),
              );
              const matchedOverlayBins =
                discreteMatchMode === "by_index"
                  ? discreteBaseSortedCanonicalIndices.map((sourceIndex) => {
                      const bin = sourceIndex >= 0 ? overlayCanonicalBins[sourceIndex] : null;
                      return bin || null;
                    })
                  : (() => {
                      return discreteBaseKeys.map((key) => {
                        const bin = overlayBinByKey.get(key);
                        return bin || null;
                      });
                    })();
              const displayOverlayBins =
                histogramMode === HISTOGRAM_MODE_CDF
                  ? buildCdfBinsPreservingNulls(matchedOverlayBins)
                  : matchedOverlayBins;
              const values = displayOverlayBins.map((bin) => {
                if (!bin) return null;
                const numeric = Number(bin?.value);
                if (!Number.isFinite(numeric)) return null;
                return yScale === "log" ? signedLog10(numeric) : numeric;
              });
              const relative = displayOverlayBins.map((bin) => {
                if (!bin) return null;
                const value = Number(bin?.value);
                const error = Number(bin?.error);
                if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
                return Math.abs(error / value);
              });
              const absoluteError = values.map((value, index) => {
                if (!Number.isFinite(value)) return null;
                const sourceBin = displayOverlayBins[index];
                const err = Number(sourceBin?.error);
                if (!Number.isFinite(err) || err <= 0) return null;
                const sourceValue = Number(sourceBin?.value);
                if (!Number.isFinite(sourceValue)) return null;
                const yLowRaw = sourceValue - Math.abs(err);
                const yHighRaw = sourceValue + Math.abs(err);
                const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
                const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
                return [index, yLow, yHigh];
              });
              return {
                id: `${bundle.id}-${selectedName}`,
                name: seriesLabel,
                color,
                discreteValues: values,
                discreteRelative: relative,
                discreteAbsError: absoluteError.filter(Boolean),
                ratioData: buildLogRatioPoints(bins, displayOverlayBins, true, effectiveXScale),
              };
            }
            const overlayCanonicalBins = buildHistogramData(histogram.bins).sort(
              (left, right) => Number(left?.start) - Number(right?.start),
            );
            const displayOverlayBins =
              histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(overlayCanonicalBins) : overlayCanonicalBins;
            const projected = projectOverlayHistogramToReferenceBins(
              bins,
              displayOverlayBins,
              yScale,
              effectiveXScale,
            );
            return {
              id: `${bundle.id}-${selectedName}`,
              name: seriesLabel,
              color,
              valueStep: projected.valueStep,
              relativeStep: projected.relativeStep,
              absError: projected.absError,
              ratioData: buildLogRatioPoints(bins, projectBinsToReferenceBins(bins, displayOverlayBins), false, effectiveXScale),
            };
          });
        })
        .filter(Boolean),
    [
      bins,
      comparedBundleSelections,
      discreteBaseKeys,
      discreteBaseSortedCanonicalIndices,
      effectiveXScale,
      histogramMode,
      isDiscrete,
      yScale,
    ],
  );
  const embeddedOverlaySeries = useMemo(
    () =>
      asArray(state?.overlay_histograms)
        .map((overlay, overlayIndex) => {
          const overlayBins = asArray(overlay?.bins);
          if (overlayBins.length === 0) return null;
          const overlayName =
            typeof overlay?.name === "string" && overlay.name.trim().length > 0
              ? overlay.name
              : `overlay_${overlayIndex + 1}`;
          const overlayColor =
            typeof overlay?.color === "string" && overlay.color.trim().length > 0
              ? overlay.color
              : histogramOverlayColors[overlayIndex % histogramOverlayColors.length];

          if (isDiscrete) {
            const overlayCanonicalBins = buildHistogramData(overlayBins);
            const overlayBinByKey = new Map(
              overlayCanonicalBins.map((bin, index) => [discreteHistogramBinKey(bin, index), bin]),
            );
            const matchedOverlayBins = discreteBaseKeys.map((key) => overlayBinByKey.get(key) || null);
            const displayOverlayBins =
              histogramMode === HISTOGRAM_MODE_CDF
                ? buildCdfBinsPreservingNulls(matchedOverlayBins)
                : matchedOverlayBins;
            const values = displayOverlayBins.map((bin) => {
              if (!bin) return null;
              const numeric = Number(bin?.value);
              if (!Number.isFinite(numeric)) return null;
              return yScale === "log" ? signedLog10(numeric) : numeric;
            });
            const relative = displayOverlayBins.map((bin) => {
              if (!bin) return null;
              const value = Number(bin?.value);
              const error = Number(bin?.error);
              if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
              return Math.abs(error / value);
            });
            const absoluteError = values.map((value, index) => {
              if (!Number.isFinite(value)) return null;
              const sourceBin = displayOverlayBins[index];
              const err = Number(sourceBin?.error);
              if (!Number.isFinite(err) || err <= 0) return null;
              const sourceValue = Number(sourceBin?.value);
              if (!Number.isFinite(sourceValue)) return null;
              const yLowRaw = sourceValue - Math.abs(err);
              const yHighRaw = sourceValue + Math.abs(err);
              const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
              const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
              return [index, yLow, yHigh];
            });
            return {
              id: `embedded-overlay-${overlayIndex}`,
              name: overlayName,
              color: overlayColor,
              discreteValues: values,
              discreteRelative: relative,
              discreteAbsError: absoluteError.filter(Boolean),
              ratioData: buildLogRatioPoints(bins, displayOverlayBins, true, effectiveXScale),
            };
          }
          const overlayCanonicalBins = buildHistogramData(overlayBins).sort(
            (left, right) => Number(left?.start) - Number(right?.start),
          );
          const displayOverlayBins =
            histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(overlayCanonicalBins) : overlayCanonicalBins;
              const useSharedEdges = state?.overlay_alignment === "shared_edges" && binsShareEdges(bins, displayOverlayBins);
              const projected =
                useSharedEdges
                  ? buildOverlaySeriesFromBins(displayOverlayBins, yScale, effectiveXScale)
                  : projectOverlayHistogramToReferenceBins(bins, displayOverlayBins, yScale, effectiveXScale);
          return {
            id: `embedded-overlay-${overlayIndex}`,
              name: overlayName,
              color: overlayColor,
              suppressErrorBars: overlay?.suppress_error_bars === true,
              valueStep: projected.valueStep,
              relativeStep: projected.relativeStep,
              absError: projected.absError,
              ratioData: buildLogRatioPoints(
                bins,
                useSharedEdges
                  ? displayOverlayBins
                  : projectBinsToReferenceBins(bins, displayOverlayBins),
              false,
              effectiveXScale,
            ),
          };
        })
        .filter(Boolean),
    [bins, discreteBaseKeys, effectiveXScale, histogramMode, isDiscrete, state?.overlay_alignment, state?.overlay_histograms, yScale],
  );
  const overlaySeries = useMemo(
    () => [...comparedOverlaySeries, ...embeddedOverlaySeries],
    [comparedOverlaySeries, embeddedOverlaySeries],
  );

  const xDomain = useMemo(() => {
    if (isDiscrete) return null;
    if (effectiveXScale !== "log") return fitHistogramXDomain(bins);
    const positiveEdges = bins
      .flatMap((bin) => [Number(bin?.start), Number(bin?.stop)])
      .filter((value) => Number.isFinite(value) && value > 0);
    if (positiveEdges.length === 0) return [Number.EPSILON, 1];
    const min = Math.min(...positiveEdges);
    const max = Math.max(...positiveEdges);
    return [Math.max(min, Number.EPSILON), Math.max(max, min * (1 + Number.EPSILON))];
  }, [bins, effectiveXScale, isDiscrete]);
  const visibleXRange = useMemo(
    () => (isDiscrete ? null : visibleXRangeFromZoomWithScale(xDomain, zoomRange, effectiveXScale)),
    [xDomain, effectiveXScale, zoomRange, isDiscrete],
  );
  const yDomain = useMemo(() => {
    const baseDomain = buildHistogramYDomain(bins, yScale, visibleXRange);
    const extraValues = [];
    for (const overlay of asArray(overlaySeries)) {
      if (isDiscrete) {
        for (const y of asArray(overlay?.discreteValues)) {
          const numeric = Number(y);
          if (Number.isFinite(numeric)) extraValues.push(numeric);
        }
        for (const entry of asArray(overlay?.discreteAbsError)) {
          const low = Number(entry?.[1]);
          const high = Number(entry?.[2]);
          if (Number.isFinite(low)) extraValues.push(low);
          if (Number.isFinite(high)) extraValues.push(high);
        }
        continue;
      }
      for (const point of asArray(overlay?.valueStep)) {
        const x = Number(point?.[0]);
        const y = Number(point?.[1]);
        if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
        if (visibleXRange && (x < visibleXRange.min || x > visibleXRange.max)) continue;
        extraValues.push(y);
      }
      for (const entry of asArray(overlay?.absError)) {
        const x = Number(entry?.[0]);
        const low = Number(entry?.[1]);
        const high = Number(entry?.[2]);
        if (!Number.isFinite(x)) continue;
        if (visibleXRange && (x < visibleXRange.min || x > visibleXRange.max)) continue;
        if (Number.isFinite(low)) extraValues.push(low);
        if (Number.isFinite(high)) extraValues.push(high);
      }
    }
    if (isDiscrete && pdfComparisonData) {
      for (const entry of asArray(pdfComparisonData.markerData)) {
        const numeric = Number(entry?.value);
        if (Number.isFinite(numeric)) extraValues.push(numeric);
      }
      for (const entry of asArray(pdfComparisonData.deltaData)) {
        const low = Number(entry?.[1]);
        const high = Number(entry?.[2]);
        if (Number.isFinite(low)) extraValues.push(low);
        if (Number.isFinite(high)) extraValues.push(high);
      }
    }
    if (extraValues.length === 0) return baseDomain;
    const baseMin = Number(baseDomain?.[0]);
    const baseMax = Number(baseDomain?.[1]);
    const candidates = [
      ...extraValues,
      ...(Number.isFinite(baseMin) ? [baseMin] : []),
      ...(Number.isFinite(baseMax) ? [baseMax] : []),
    ].filter((value) => Number.isFinite(value));
    return fitDomain(candidates);
  }, [bins, isDiscrete, overlaySeries, pdfComparisonData, visibleXRange, yScale]);
  const relativeErrorYDomain = useMemo(
    () => buildRelativeErrorYDomain(isDiscrete ? discreteRelativeErrorData : relativeErrorData, visibleXRange),
    [discreteRelativeErrorData, isDiscrete, relativeErrorData, visibleXRange],
  );
  const binErrorData = useMemo(
    () =>
      bins
        .map((bin, index) => {
          const x = isDiscrete ? null : Number(bin?.x);
          const y = Number(bin?.value);
          const err = Number(bin?.error);
          if (isDiscrete) {
            if (!Number.isFinite(y) || !Number.isFinite(err) || err <= 0) return null;
            const yLowRaw = y - Math.abs(err);
            const yHighRaw = y + Math.abs(err);
            const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
            const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
            return [index, yLow, yHigh];
          }
          if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(err) || err <= 0) {
            return null;
          }
          const yLowRaw = y - Math.abs(err);
          const yHighRaw = y + Math.abs(err);
          const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
          const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
          if (effectiveXScale === "log" && x <= 0) return null;
          return [x, yLow, yHigh];
        })
        .filter(Boolean),
    [bins, effectiveXScale, yScale, isDiscrete],
  );
  const histogramOption = useMemo(
    () =>
      buildHistogramOption({
        binErrorData,
        bins,
        categories,
        effectiveXScale,
        isDiscrete,
        metricDescriptors,
        overlaySeries,
        panelId,
        pdfComparisonData,
        selectedView,
        stepData,
        xDomain,
        yDomain,
        yScale,
        yZoomRange,
        zoomRange,
      }),
    [
      binErrorData,
      bins,
      categories,
      effectiveXScale,
      isDiscrete,
      metricDescriptors,
      overlaySeries,
      panelId,
      pdfComparisonData,
      selectedView,
      stepData,
      xDomain,
      yDomain,
      yScale,
      yZoomRange,
      zoomRange,
    ],
  );

  const relativeOption = useMemo(
    () =>
      buildRelativeHistogramOption({
        bins,
        categories,
        discreteRelativeErrorData,
        effectiveXScale,
        isDiscrete,
        overlaySeries,
        panelId,
        relativeErrorData,
        relativeErrorYDomain,
        xDomain,
        yZoomRange,
        zoomRange,
      }),
    [
      bins,
      categories,
      discreteRelativeErrorData,
      effectiveXScale,
      isDiscrete,
      overlaySeries,
      panelId,
      relativeErrorData,
      relativeErrorYDomain,
      xDomain,
      yZoomRange,
      zoomRange,
    ],
  );

  const ratioOption = useMemo(
    () =>
      buildRatioHistogramOption({
        bins,
        categories,
        effectiveXScale,
        fitDomain,
        isDiscrete,
        overlaySeries,
        panelId,
        xDomain,
        yZoomRange,
        zoomRange,
      }),
    [bins, categories, effectiveXScale, isDiscrete, overlaySeries, panelId, xDomain, yZoomRange, zoomRange],
  );

  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        const next = readDataZoomRanges(event);
        if (!next || typeof onValueChange !== "function" || !sourcePanelId) return;
        const nextX = next.x || zoomRange;
        const nextY = next.y || yZoomRange;
        const xChanged = Boolean(next.x) && zoomRangeChanged(zoomRange, nextX);
        const yChanged = Boolean(next.y) && zoomRangeChanged(yZoomRange, nextY);
        if (!xChanged && !yChanged) return;
        onValueChange(
          sourcePanelId,
          isBundleControlled
            ? writeHistogramBundlePanelValue(value, { zoom: nextX, yZoom: nextY })
            : writeZoomPanelValue(value, nextX, null, nextY),
          false,
        );
      },
    }),
    [isBundleControlled, onValueChange, sourcePanelId, value, yZoomRange, zoomRange],
  );

  if (bins.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">
            {title}
            {state?.name ? `  (${state.name})` : ""}
          </Typography>
          <Stack direction="row" spacing={1} alignItems="center">
            {showExportAction || showResetViewAction ? (
            <FigureExportActions
              baseName={state?.panel_id || state?.name || title || "histogram"}
              payload={{ panel_id: state?.panel_id ?? null, kind: "histogram", state, xScale: effectiveXScale, yScale }}
              elementRef={figureRef}
              onResetView={
                showResetViewAction && sourcePanelId && typeof onValueChange === "function"
                  ? () =>
                      onValueChange(
                        sourcePanelId,
                        isBundleControlled
                          ? writeHistogramBundlePanelValue(value, { zoom: FULL_ZOOM, yZoom: FULL_ZOOM })
                          : writeZoomPanelValue(value, FULL_ZOOM, null, FULL_ZOOM),
                        false,
                      )
                : null
              }
            />
            ) : null}
            {declaredViews.length > 1 ? (
              <FormControl size="small" sx={{ minWidth: 152 }}>
                <Select
                  value={selectedView.id}
                  onChange={(event) => {
                    const next = String(event.target.value || "");
                    if (isBundleControlled && sourcePanelId && typeof onValueChange === "function" && currentHistogramName) {
                      onValueChange(
                        sourcePanelId,
                        writeHistogramBundlePanelValue(value, {
                          selectedViewByHistogram: { [currentHistogramName]: next },
                        }),
                        false,
                      );
                      return;
                    }
                    setLocalSelectedViewId(next);
                  }}
                  sx={{
                    fontSize: "0.875rem",
                    ".MuiSelect-select": { py: 0.75 },
                  }}
                >
                  {declaredViews.map((view) => (
                    <MenuItem key={view.id} value={view.id}>
                      {view.label}
                    </MenuItem>
                  ))}
                </Select>
              </FormControl>
            ) : null}
            {showScaleControls ? (
            <FormControl size="small" sx={{ minWidth: 128 }}>
              <Select
                value={yScale}
                onChange={(event) => {
                  const next = event.target.value;
                  if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                    onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { yScale: next }), false);
                    return;
                  }
                  setLocalYScale(next);
                }}
                sx={{
                  fontSize: "0.875rem",
                  ".MuiSelect-select": { py: 0.75 },
                }}
              >
                <MenuItem value="linear">Y Linear</MenuItem>
                <MenuItem value="log">Y Log</MenuItem>
              </Select>
            </FormControl>
            ) : null}
            {showXScaleControl ? (
            <FormControl size="small" sx={{ minWidth: 128 }}>
              <Select
                value={effectiveXScale}
                disabled={isDiscrete}
                onChange={(event) => {
                  const next = event.target.value;
                  if (isDiscrete) {
                    return;
                  }
                  if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                    onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { xScale: next }), false);
                    return;
                  }
                  setLocalXScale(next);
                }}
                sx={{
                  fontSize: "0.875rem",
                  ".MuiSelect-select": { py: 0.75 },
                }}
              >
                <MenuItem value="linear">X Linear</MenuItem>
                <MenuItem value="log">X Log</MenuItem>
              </Select>
            </FormControl>
            ) : null}
            {showPdfCdfControl ? (
            <Button
              size="small"
              variant={histogramMode === HISTOGRAM_MODE_CDF ? "contained" : "outlined"}
              onClick={() => {
                const next = histogramMode === HISTOGRAM_MODE_CDF ? HISTOGRAM_MODE_PDF : HISTOGRAM_MODE_CDF;
                if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                  onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { histogramMode: next }), false);
                  return;
                }
                setLocalHistogramMode(next);
              }}
            >
              {histogramMode === HISTOGRAM_MODE_CDF ? "CDF" : "PDF"}
            </Button>
            ) : null}
            {showRatioControl && ratioOption ? (
            <Button
              size="small"
              variant={showRatio ? "contained" : "outlined"}
              onClick={() => {
                const next = !showRatio;
                if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                  onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { showRatio: next }), false);
                  return;
                }
                setLocalShowRatio(next);
              }}
            >
              Ratio
            </Button>
            ) : null}
            {isDiscrete && showSortControl ? (
              <FormControl size="small" sx={{ minWidth: 156 }}>
                <Select
                  value={requestedSortMode}
                  onChange={(event) => {
                    const next = normalizeHistogramSortMode(event.target.value);
                    if (
                      isBundleControlled &&
                      sourcePanelId &&
                      typeof onValueChange === "function" &&
                      currentHistogramName
                    ) {
                      onValueChange(
                        sourcePanelId,
                        writeHistogramBundlePanelValue(value, {
                          sortModeByHistogram: { [currentHistogramName]: next },
                        }),
                        false,
                      );
                      return;
                    }
                    setLocalSortMode(next);
                  }}
                  sx={{
                    fontSize: "0.875rem",
                    ".MuiSelect-select": { py: 0.75 },
                  }}
                >
                  <MenuItem value={HISTOGRAM_SORT_CANONICAL}>Lexicographic</MenuItem>
                  <MenuItem value={HISTOGRAM_SORT_BY_VALUE}>By Value</MenuItem>
                  <MenuItem value={HISTOGRAM_SORT_BY_ABS_VALUE}>By |Value|</MenuItem>
                </Select>
              </FormControl>
            ) : null}
            {showRelativeErrorControl ? (
            <Button
              size="small"
              variant={showRelativeErrors ? "contained" : "outlined"}
              onClick={() => {
                const next = !showRelativeErrors;
                if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                  onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { showRelativeError: next }), false);
                  return;
                }
                setLocalShowRelativeErrors(next);
              }}
            >
              Rel Error
            </Button>
            ) : null}
          </Stack>
        </Box>
        {comparedBundleSelections.length > 0 ? (
          <Box sx={{ mb: 1.5, display: "grid", gap: 1 }}>
            {comparedBundleSelections.map(({ bundle, selectionState }) => {
              const histogramNames = Object.keys(bundle.histograms || {}).filter(
                (name) => histogramIsDiscrete(bundle.histograms?.[name]) === isDiscrete,
              );
              const usedNames = new Set(
                asArray(selectionState.selectedHistograms).filter((value) => typeof value === "string"),
              );
              const hasAddableComparison = histogramNames.some((name) => !usedNames.has(name));
              return (
                <Box
                  key={bundle.id}
                  sx={{
                    display: "grid",
                    gap: 1,
                    border: "1px solid",
                    borderColor: "divider",
                    borderRadius: 1,
                    px: 1,
                    py: 0.75,
                  }}
                >
                  <Stack direction="row" spacing={1} alignItems="center" sx={{ flexWrap: "wrap", rowGap: 0.75 }}>
                    <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0, mr: 0.5 }}>
                      {bundle.label}
                    </Typography>
                    {isDiscrete ? (
                      <Typography variant="caption" color="text.secondary">
                        Discrete matching is configured per histogram row.
                      </Typography>
                    ) : null}
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={!hasAddableComparison}
                      onClick={() =>
                        onAddComparedHistogram?.(sourcePanelId, bundle.id, currentHistogramName, isDiscrete)
                      }
                    >
                      Add Histogram
                    </Button>
                  </Stack>
                  <Box sx={{ display: "grid", gap: 0.75 }}>
                    {selectionState.selectedHistograms.map((comparedName, comparedIndex) => (
                      <Box
                        key={`${bundle.id}-${comparedIndex}`}
                        sx={{
                          display: "grid",
                          gridTemplateColumns: {
                            xs: "1fr auto",
                            sm: isDiscrete ? "minmax(0, 200px) 150px 150px auto" : "minmax(0, 220px) auto",
                          },
                          gap: 0.75,
                          alignItems: "center",
                        }}
                      >
                        <FormControl size="small">
                          <Select
                            value={comparedName}
                            onChange={(event) => {
                              const nextSelectedHistograms = selectionState.selectedHistograms.slice();
                              nextSelectedHistograms[comparedIndex] = String(event.target.value || "");
                              const nextDiscreteAlignmentByHistogram = {
                                ...(selectionState.discreteAlignmentByHistogram || {}),
                              };
                              const nextSortModeByHistogram = {
                                ...(selectionState.sortModeByHistogram || {}),
                              };
                              const oldName = selectionState.selectedHistograms[comparedIndex];
                              const nextName = nextSelectedHistograms[comparedIndex];
                              if (oldName && oldName !== nextName && !nextSelectedHistograms.includes(oldName)) {
                                delete nextDiscreteAlignmentByHistogram[oldName];
                                delete nextSortModeByHistogram[oldName];
                              }
                              if (nextName && !nextDiscreteAlignmentByHistogram[nextName]) {
                                nextDiscreteAlignmentByHistogram[nextName] = "by_key";
                              }
                              if (nextName && !nextSortModeByHistogram[nextName]) {
                                nextSortModeByHistogram[nextName] = HISTOGRAM_SORT_CANONICAL;
                              }
                              onUpdateBundleSelection?.(
                                sourcePanelId,
                                bundle.id,
                                currentHistogramName,
                                nextSelectedHistograms,
                                nextDiscreteAlignmentByHistogram,
                                nextSortModeByHistogram,
                              );
                            }}
                            sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                          >
                            {histogramNames.map((name) => (
                              <MenuItem key={`${bundle.id}-${comparedIndex}-${name}`} value={name}>
                                {name}
                              </MenuItem>
                            ))}
                          </Select>
                        </FormControl>
                        {isDiscrete ? (
                          <FormControl size="small">
                            <Select
                              value={normalizeHistogramSortMode(selectionState.sortModeByHistogram?.[comparedName])}
                              onChange={(event) => {
                                const nextSortModeByHistogram = {
                                  ...(selectionState.sortModeByHistogram || {}),
                                  [comparedName]: normalizeHistogramSortMode(event.target.value),
                                };
                                onUpdateBundleSelection?.(
                                  sourcePanelId,
                                  bundle.id,
                                  currentHistogramName,
                                  selectionState.selectedHistograms,
                                  selectionState.discreteAlignmentByHistogram,
                                  nextSortModeByHistogram,
                                );
                              }}
                              sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                            >
                              <MenuItem value={HISTOGRAM_SORT_CANONICAL}>Canonical</MenuItem>
                              <MenuItem value={HISTOGRAM_SORT_BY_VALUE}>By Value</MenuItem>
                              <MenuItem value={HISTOGRAM_SORT_BY_ABS_VALUE}>By |Value|</MenuItem>
                            </Select>
                          </FormControl>
                        ) : null}
                        {isDiscrete ? (
                          <FormControl size="small">
                            <Select
                              value={
                                selectionState.discreteAlignmentByHistogram?.[comparedName] === "by_index"
                                  ? "by_index"
                                  : "by_key"
                              }
                              onChange={(event) => {
                                const nextDiscreteAlignmentByHistogram = {
                                  ...(selectionState.discreteAlignmentByHistogram || {}),
                                  [comparedName]: event.target.value === "by_index" ? "by_index" : "by_key",
                                };
                                onUpdateBundleSelection?.(
                                  sourcePanelId,
                                  bundle.id,
                                  currentHistogramName,
                                  selectionState.selectedHistograms,
                                  nextDiscreteAlignmentByHistogram,
                                  selectionState.sortModeByHistogram,
                                );
                              }}
                              sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                            >
                              <MenuItem value="by_key">Match: Key</MenuItem>
                              <MenuItem value="by_index">Match: Index</MenuItem>
                            </Select>
                          </FormControl>
                        ) : null}
                        <Button
                          size="small"
                          variant="text"
                          color="error"
                          onClick={() =>
                            onRemoveComparedHistogram?.(sourcePanelId, bundle.id, currentHistogramName, comparedIndex)
                          }
                        >
                          Remove
                        </Button>
                      </Box>
                    ))}
                  </Box>
                </Box>
              );
            })}
          </Box>
        ) : null}
        <Box ref={figureRef} sx={{ width: "100%", display: "grid", gap: 2 }}>
          <Box sx={{ width: "100%", height: 280 }}>
            <LazyChart
              ref={echartsRef}
              option={histogramOption}
              notMerge={!isDiscrete}
              onEvents={onDataZoom}
              lazyUpdate
              opts={{ renderer: "canvas" }}
              style={{ width: "100%", height: "100%" }}
            />
          </Box>
          {showRelativeErrors && relativeOption ? (
            <Box>
              <Typography variant="caption" color="text.secondary">
                Relative Error Shape
              </Typography>
              <Box sx={{ width: "100%", height: 168 }}>
                <LazyChart
                  option={relativeOption}
                  notMerge
                  onEvents={onDataZoom}
                  lazyUpdate
                  opts={{ renderer: "canvas" }}
                  style={{ width: "100%", height: "100%" }}
                />
              </Box>
            </Box>
          ) : null}
          {showRatio && ratioOption ? (
            <Box>
              <Typography variant="caption" color="text.secondary">
                Histogram Ratio: log10(value / comparison), agreement at 0
              </Typography>
              <Box sx={{ width: "100%", height: 188 }}>
                <LazyChart
                  option={ratioOption}
                  notMerge
                  onEvents={onDataZoom}
                  lazyUpdate
                  opts={{ renderer: "canvas" }}
                  style={{ width: "100%", height: "100%" }}
                />
              </Box>
            </Box>
          ) : null}
        </Box>
      </CardContent>
    </Card>
  );
};


export default HistogramPanel;
