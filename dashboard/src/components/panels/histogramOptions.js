import { asArray } from "../../utils/collections";
import { formatScientific } from "../../utils/formatters";
import { escapeXml } from "./FigureExportActions";
import {
  HISTOGRAM_NEGATIVE_COLOR,
  HISTOGRAM_POSITIVE_COLOR,
  HISTOGRAM_ZERO_COLOR,
  buildHistogramRenderData,
  buildRelativeErrorStepData,
  formatSignedLogAxisValue,
  histogramSignColorFromRaw,
  signedLog10,
} from "./histogramUtils";
import { buildDataZoom } from "./panelView";
import { discreteBinInfoLines } from "./histogramViews";

const gridColor = "rgba(148,163,184,0.18)";
const DISCRETE_BAR_CATEGORY_GAP = "30%";
const DISCRETE_BAR_GAP = "30%";

const baseCartesianGrid = {
  left: 56,
  right: 20,
  top: 12,
  bottom: 48,
};

const formatAxisValue = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? formatScientific(numeric, 3) : "";
};

const baseAxisLabel = {
  color: "#64748b",
  fontSize: 12,
  formatter: (value) => formatAxisValue(value),
};

const inferXAxisLabel = (panelId) => (String(panelId || "").includes("_history") ? "Nr samples" : null);
const inferNumericXAxisLabel = (panelId) => inferXAxisLabel(panelId) || "x";

const formatCategoryAxisValue = (value) => {
  if (value == null) return "";
  const text = String(value).trim();
  return text.length > 0 ? text : "";
};

const histogramEntryCountLine = (bin) => {
  const entryCount = Number(bin?.entry_count);
  if (!Number.isFinite(entryCount)) return null;
  return `entries: ${formatScientific(entryCount, 6)}`;
};

const EDGE_EPSILON = 1e-9;

const nearlyEqual = (left, right, epsilon = EDGE_EPSILON) => {
  const a = Number(left);
  const b = Number(right);
  if (!Number.isFinite(a) || !Number.isFinite(b)) return false;
  return Math.abs(a - b) <= epsilon * Math.max(1, Math.abs(a), Math.abs(b));
};

export const binsShareEdges = (referenceBins, overlayBins) => {
  if (referenceBins.length !== overlayBins.length) return false;
  for (let i = 0; i < referenceBins.length; i += 1) {
    const left = referenceBins[i];
    const right = overlayBins[i];
    if (!nearlyEqual(left?.start, right?.start) || !nearlyEqual(left?.stop, right?.stop)) return false;
  }
  return true;
};

export const buildOverlaySeriesFromBins = (canonicalBins, yScale, xScale) => ({
  valueStep: buildHistogramRenderData(canonicalBins, yScale)
    .map((point) => [Number(point?.x), Number(point?.y)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0)),
  relativeStep: buildRelativeErrorStepData(canonicalBins)
    .map((point) => [Number(point?.x), Number(point?.relative_error)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0)),
  absError: canonicalBins
    .map((bin) => {
      const x = Number(bin?.x);
      const y = Number(bin?.value);
      const err = Number(bin?.error);
      if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(err) || err <= 0) return null;
      const yLowRaw = y - Math.abs(err);
      const yHighRaw = y + Math.abs(err);
      const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
      const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
      if (xScale === "log" && x <= 0) return null;
      return [x, yLow, yHigh];
    })
    .filter(Boolean),
});

const sampleHistogramBinAtX = (bins, x) => {
  const numericX = Number(x);
  if (!Number.isFinite(numericX)) return null;
  for (const bin of asArray(bins)) {
    const start = Number(bin?.start);
    const stop = Number(bin?.stop);
    if (!Number.isFinite(start) || !Number.isFinite(stop)) continue;
    const contains = numericX >= start - EDGE_EPSILON && numericX <= stop + EDGE_EPSILON;
    if (contains) return bin;
  }
  return null;
};

export const projectBinsToReferenceBins = (referenceBins, overlayBins) =>
  asArray(referenceBins).map((referenceBin) => {
    const matched = sampleHistogramBinAtX(overlayBins, referenceBin?.x);
    if (!matched) return null;
    return {
      start: referenceBin.start,
      stop: referenceBin.stop,
      x: referenceBin.x,
      value: Number(matched.value),
      error: Number.isFinite(Number(matched.error)) ? Math.abs(Number(matched.error)) : 0,
    };
  });

export const buildCdfBinsPreservingNulls = (bins) => {
  let cumulativeValue = 0;
  let cumulativeVariance = 0;
  return asArray(bins).map((bin) => {
    if (!bin) return null;
    const value = Number(bin?.value);
    const error = Math.abs(Number(bin?.error));
    if (Number.isFinite(value)) cumulativeValue += value;
    if (Number.isFinite(error)) cumulativeVariance += error * error;
    return {
      ...bin,
      value: cumulativeValue,
      error: Math.sqrt(cumulativeVariance),
    };
  });
};

export const buildLogRatioPoints = (referenceBins, overlayBins, isDiscrete, xScale) =>
  asArray(referenceBins)
    .map((referenceBin, index) => {
      const overlayBin = asArray(overlayBins)[index];
      const numerator = Number(referenceBin?.value);
      const denominator = Number(overlayBin?.value);
      const numeratorError = Math.abs(Number(referenceBin?.error));
      const denominatorError = Math.abs(Number(overlayBin?.error));
      if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || numerator === 0 || denominator === 0) {
        return null;
      }
      const ratio = numerator / denominator;
      if (!Number.isFinite(ratio) || ratio <= 0) return null;
      const relativeNumeratorError =
        Number.isFinite(numeratorError) && numerator !== 0 ? numeratorError / Math.abs(numerator) : 0;
      const relativeDenominatorError =
        Number.isFinite(denominatorError) && denominator !== 0 ? denominatorError / Math.abs(denominator) : 0;
      const logRatio = Math.log10(ratio);
      const logRatioError = Math.hypot(relativeNumeratorError, relativeDenominatorError) / Math.LN10;
      const x = isDiscrete ? index : Number(referenceBin?.x);
      if (!Number.isFinite(x) || (!isDiscrete && xScale === "log" && x <= 0)) return null;
      return [x, logRatio, logRatio - logRatioError, logRatio + logRatioError];
    })
    .filter(Boolean);

const buildErrorBarSeries = ({ name = "error", data, color = "#7c8a96", capPx = 4 }) => ({
  type: "custom",
  name,
  data,
  clip: true,
  silent: true,
  z: 5,
  tooltip: { show: false },
  renderItem: (params, api) => {
    const xValue = Number(api.value(0));
    const yLowValue = Number(api.value(1));
    const yHighValue = Number(api.value(2));
    if (!Number.isFinite(xValue) || !Number.isFinite(yLowValue) || !Number.isFinite(yHighValue)) return null;
    const [xPx, yLowPx] = api.coord([xValue, yLowValue]);
    const [, yHighPx] = api.coord([xValue, yHighValue]);
    if (!Number.isFinite(xPx) || !Number.isFinite(yLowPx) || !Number.isFinite(yHighPx)) return null;
    const coordSys = params?.coordSys;
    if (!coordSys) return null;
    const left = Number(coordSys.x);
    const right = Number(coordSys.x) + Number(coordSys.width);
    const top = Number(coordSys.y);
    const bottom = Number(coordSys.y) + Number(coordSys.height);
    if (!Number.isFinite(left) || !Number.isFinite(right) || !Number.isFinite(top) || !Number.isFinite(bottom)) {
      return null;
    }
    if (xPx < left || xPx > right) return null;
    if ((yLowPx < top && yHighPx < top) || (yLowPx > bottom && yHighPx > bottom)) return null;
    const y1 = Math.max(top, Math.min(bottom, yLowPx));
    const y2 = Math.max(top, Math.min(bottom, yHighPx));
    const capLeft = Math.max(left, xPx - capPx);
    const capRight = Math.min(right, xPx + capPx);
    return {
      type: "group",
      children: [
        {
          type: "line",
          shape: { x1: xPx, y1, x2: xPx, y2 },
          style: { stroke: color, lineWidth: 1.2 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1, x2: capRight, y2: y1 },
          style: { stroke: color, lineWidth: 1.2 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1: y2, x2: capRight, y2 },
          style: { stroke: color, lineWidth: 1.2 },
        },
      ],
    };
  },
});

export const buildHistogramOption = ({
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
}) => {
    const pdfComparisonVisible = isDiscrete && pdfComparisonData;
    const legendEntries = [
      "value",
      ...overlaySeries.map((overlay) => overlay.name),
      ...(pdfComparisonVisible ? [pdfComparisonData.markerLabel, pdfComparisonData.deltaLabel] : []),
    ];
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      const barData = bins.map((bin) => {
        const numericValue = Number(bin?.value);
        if (!Number.isFinite(numericValue)) return null;
        return {
          value: yScale === "log" ? signedLog10(numericValue) : numericValue,
          rawValue: numericValue,
        };
      });
      const discreteBarSeriesCount = 1 + overlaySeries.length;
      return {
        animation: false,
        legend: {
          show: true,
          top: 0,
          data: legendEntries,
          textStyle: { color: "#64748b", fontSize: 12 },
        },
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: yDomain[0],
          max: yDomain[1],
          axisLabel: yScale === "log" ? { ...baseAxisLabel, formatter: formatSignedLogAxisValue } : baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "item",
          formatter: (params) => {
            const idx = params.dataIndex;
            const label = categoriesData[idx] ?? String(idx);
            const bin = bins[idx];
            const val = Number(bin?.value);
            const err = Number(bin?.error);
            const absErrorText =
              Number.isFinite(err) && err > 0 ? `±${formatScientific(err, 6)}` : "n/a";
            const relError =
              Number.isFinite(val) && Number.isFinite(err) && val !== 0
                ? Math.abs(err / val)
                : null;
            const relErrorText = Number.isFinite(relError)
              ? formatScientific(relError, 6)
              : "n/a";
            return [
              `${escapeXml(label)}: ${formatScientific(val, 6)}`,
              histogramEntryCountLine(bin),
              `abs error: ${absErrorText}`,
              `rel error: ${relErrorText}`,
              ...discreteBinInfoLines(bin, selectedView, metricDescriptors),
            ]
              .filter(Boolean)
              .join("<br/>");
          },
        },
        dataZoom: buildDataZoom(zoomRange, false, true, yZoomRange, true),
        series: [
          ...(Array.isArray(binErrorData) && binErrorData.length > 0
            ? [
                buildDiscreteOffsetErrorBarSeries({
                  name: "error",
                  data: binErrorData,
                  slotIndex: 0,
                  slotCount: discreteBarSeriesCount,
                }),
              ]
            : []),
          {
            type: "bar",
            name: "value",
            data: barData,
            barCategoryGap: DISCRETE_BAR_CATEGORY_GAP,
            barGap: DISCRETE_BAR_GAP,
            itemStyle:
              yScale === "log"
                ? {
                    color: (params) => histogramSignColorFromRaw(params?.data?.rawValue),
                  }
                : { color: "#005f73" },
          },
          ...overlaySeries.flatMap((overlay, index) => {
            const slotIndex = index + 1;
            const series = [
              {
                type: "bar",
                name: overlay.name,
                data: overlay.discreteValues,
                barCategoryGap: DISCRETE_BAR_CATEGORY_GAP,
                barGap: DISCRETE_BAR_GAP,
                itemStyle: { color: overlay.color, opacity: 0.52 },
                emphasis: { focus: "series" },
              },
            ];
            if (
              overlay.suppressErrorBars !== true &&
              Array.isArray(overlay.discreteAbsError) &&
              overlay.discreteAbsError.length > 0
            ) {
              series.push(
                buildDiscreteOffsetErrorBarSeries({
                  name: `${overlay.name} error`,
                  data: overlay.discreteAbsError,
                  color: overlay.color,
                  slotIndex,
                  slotCount: discreteBarSeriesCount,
                }),
              );
            }
            return series;
          }),
          ...(pdfComparisonVisible
            ? [
                buildDiscreteOffsetErrorBarSeries({
                  name: pdfComparisonData.deltaLabel,
                  data: pdfComparisonData.deltaData,
                  color: "#475569",
                  slotIndex: 0,
                  slotCount: discreteBarSeriesCount,
                  barWidthRatio: 0.32,
                }),
                {
                  type: "scatter",
                  name: pdfComparisonData.markerLabel,
                  data: pdfComparisonData.markerData,
                  symbol: "diamond",
                  symbolSize: 9,
                  itemStyle: { color: "#0f172a", borderColor: "#ffffff", borderWidth: 1 },
                  z: 8,
                  tooltip: {
                    valueFormatter: (value) =>
                      Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a",
                  },
                },
              ]
            : []),
        ],
      };
    }

    const stepSeriesPoints = stepData
      .map((point) => ({
        x: Number(point?.x),
        y: Number(point?.y),
        rawY: Number(point?.raw_y ?? point?.y),
      }))
      .filter(
        (point) => Number.isFinite(point.x) && Number.isFinite(point.y) && (effectiveXScale !== "log" || point.x > 0),
      );
    const valueSeriesData = stepSeriesPoints.map((point) => [point.x, point.y]);
    const positiveSeriesData =
      yScale === "log"
        ? stepSeriesPoints.map((point) => (point.rawY > 0 ? [point.x, point.y] : [point.x, null]))
        : valueSeriesData;
    const negativeSeriesData =
      yScale === "log" ? stepSeriesPoints.map((point) => (point.rawY < 0 ? [point.x, point.y] : [point.x, null])) : [];
    const zeroSeriesData =
      yScale === "log"
        ? stepSeriesPoints.map((point) => (point.rawY === 0 ? [point.x, point.y] : [point.x, null]))
        : [];
    const valueSeries =
      yScale === "log"
        ? [
            {
              id: "histogram-value-positive",
              type: "line",
              name: "value (+)",
              data: positiveSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_POSITIVE_COLOR },
              itemStyle: { color: HISTOGRAM_POSITIVE_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
            {
              id: "histogram-value-negative",
              type: "line",
              name: "value (-)",
              data: negativeSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_NEGATIVE_COLOR },
              itemStyle: { color: HISTOGRAM_NEGATIVE_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
            {
              id: "histogram-value-zero",
              type: "line",
              name: "value (0)",
              data: zeroSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_ZERO_COLOR },
              itemStyle: { color: HISTOGRAM_ZERO_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
          ]
        : [
            {
              id: "histogram-value",
              type: "line",
              name: "value",
              data: valueSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: "#005f73" },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
          ];
    return {
      animation: false,
      legend: {
        show: true,
        top: 0,
        data: legendEntries,
        textStyle: { color: "#64748b", fontSize: 12 },
      },
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: yDomain[0],
        max: yDomain[1],
        axisLabel: yScale === "log" ? { ...baseAxisLabel, formatter: formatSignedLogAxisValue } : baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        formatter: (params) => {
          const entries = asArray(params);
          const xValue = Number(entries[0]?.axisValue);
          if (!Number.isFinite(xValue)) return "n/a";
          const bin = bins.find((candidate) => {
            const start = Number(candidate?.start);
            const stop = Number(candidate?.stop);
            if (!Number.isFinite(start) || !Number.isFinite(stop)) return false;
            const inclusiveStop = Math.abs(xValue - stop) <= 1e-12;
            return xValue >= start && (xValue < stop || inclusiveStop);
          });
          if (!bin) {
            return `${formatScientific(xValue, 6)}: n/a`;
          }
          const valueNumeric = Number(bin?.value);
          const errorNumeric = Number(bin?.error);
          const absErrorText =
            Number.isFinite(errorNumeric) && errorNumeric > 0 ? `±${formatScientific(errorNumeric, 6)}` : "n/a";
          const relError =
            Number.isFinite(valueNumeric) && Number.isFinite(errorNumeric) && valueNumeric !== 0
              ? Math.abs(errorNumeric / valueNumeric)
              : null;
          const relErrorText = Number.isFinite(relError) ? formatScientific(relError, 6) : "n/a";
          return [
            `${escapeXml(`${formatScientific(Number(bin?.start), 6)} → ${formatScientific(Number(bin?.stop), 6)}`)}: ${formatScientific(valueNumeric, 6)}`,
            histogramEntryCountLine(bin),
            `abs error: ${absErrorText}`,
            `rel error: ${relErrorText}`,
          ]
            .filter(Boolean)
            .join("<br/>");
        },
      },
      dataZoom: buildDataZoom(zoomRange, false, true, yZoomRange, true),
      series: [
        ...(Array.isArray(binErrorData) && binErrorData.length > 0
          ? [buildErrorBarSeries({ name: "error", data: binErrorData })]
          : []),
        ...valueSeries,
        ...overlaySeries.flatMap((overlay) => [
          ...(overlay.suppressErrorBars !== true && Array.isArray(overlay.absError) && overlay.absError.length > 0
            ? [buildErrorBarSeries({ name: `${overlay.name} error`, data: overlay.absError, color: overlay.color })]
            : []),
          {
            id: `histogram-overlay-${overlay.id}`,
            type: "line",
            name: overlay.name,
            data: overlay.valueStep,
            step: "end",
            showSymbol: false,
            connectNulls: false,
            lineStyle: { width: 1.4, type: "dashed", color: overlay.color },
            itemStyle: { color: overlay.color },
            emphasis: { disabled: true },
            tooltip: { show: false },
          },
        ]),
      ],
    };
};

export const buildRelativeHistogramOption = ({
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
}) => {
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      const discreteBarSeriesCount = 1 + overlaySeries.length;
      return {
        animation: false,
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: relativeErrorYDomain[0],
          max: relativeErrorYDomain[1],
          axisLabel: baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "axis",
          valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
        },
        dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
        series: [
          buildDiscreteOffsetRangeBarSeries({
            name: "relative_error",
            data: discreteRelativeErrorData
              .map((point) => {
                const rel = Number(point?.relative_error);
                if (!Number.isFinite(rel)) return null;
                return [Number(point?.index), -Math.abs(rel), Math.abs(rel)];
              })
              .filter(Boolean),
            color: "rgba(187, 62, 3, 0.55)",
            slotIndex: 0,
            slotCount: discreteBarSeriesCount,
          }),
          ...overlaySeries.map((overlay, index) =>
            buildDiscreteOffsetRangeBarSeries({
              name: overlay.name,
              data: overlay.discreteRelative
                .map((rel, relIndex) => {
                  const numeric = Number(rel);
                  if (!Number.isFinite(numeric)) return null;
                  return [relIndex, -Math.abs(numeric), Math.abs(numeric)];
                })
                .filter(Boolean),
              color: overlay.color,
              slotIndex: index + 1,
              slotCount: discreteBarSeriesCount,
            }),
          ),
        ],
      };
    }
    if (!xDomain) {
      return null;
    }
    return {
      animation: false,
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: relativeErrorYDomain[0],
        max: relativeErrorYDomain[1],
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        {
          type: "line",
          name: "positive_relative_error",
          data: relativeErrorData
            .map((point) => [Number(point?.x), Number(point?.positive_relative_error)])
            .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0)),
          step: "end",
          showSymbol: false,
          lineStyle: { width: 1.2, color: "#bb3e03" },
          areaStyle: { color: "rgba(187, 62, 3, 0.22)" },
          connectNulls: false,
        },
        {
          type: "line",
          name: "negative_relative_error",
          data: relativeErrorData
            .map((point) => [Number(point?.x), Number(point?.negative_relative_error)])
            .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0)),
          step: "end",
          showSymbol: false,
          lineStyle: { width: 1.2, color: "#bb3e03" },
          areaStyle: { color: "rgba(187, 62, 3, 0.22)" },
          connectNulls: false,
        },
        ...overlaySeries.map((overlay) => ({
          type: "line",
          name: overlay.name,
          data: overlay.relativeStep,
          step: "end",
          showSymbol: false,
          connectNulls: false,
          lineStyle: { width: 1.35, type: "dashed", color: overlay.color },
          itemStyle: { color: overlay.color },
          emphasis: { focus: "series" },
        })),
      ],
    };
};

export const buildRatioHistogramOption = ({
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
}) => {
    const ratioSeries = overlaySeries
      .map((overlay) => ({
        ...overlay,
        ratioData: asArray(overlay?.ratioData).filter((point) => point && point.every((value) => Number.isFinite(Number(value)))),
      }))
      .filter((overlay) => overlay.ratioData.length > 0);
    if (ratioSeries.length === 0) return null;
    const ratioValues = ratioSeries.flatMap((overlay) =>
      overlay.ratioData.flatMap((point) => [Number(point?.[1]), Number(point?.[2]), Number(point?.[3])]),
    );
    const ratioYDomain = fitDomain([...ratioValues, 0]);
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      return {
        animation: false,
        legend: {
          show: true,
          top: 0,
          data: ratioSeries.map((overlay) => overlay.name),
          textStyle: { color: "#64748b", fontSize: 12 },
        },
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: ratioYDomain[0],
          max: ratioYDomain[1],
          name: "log10(value / comparison)",
          axisLabel: baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "axis",
          valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
        },
        dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
        series: ratioSeries.flatMap((overlay, index) => [
          buildDiscreteOffsetErrorBarSeries({
            name: `${overlay.name} ratio error`,
            data: overlay.ratioData.map((point) => [point[0], point[2], point[3]]),
            color: overlay.color,
            slotIndex: index,
            slotCount: ratioSeries.length,
          }),
          {
            type: "line",
            name: overlay.name,
            data: overlay.ratioData.map((point) => [point[0], point[1]]),
            showSymbol: true,
            symbolSize: 4,
            lineStyle: { width: 1.35, color: overlay.color },
            itemStyle: { color: overlay.color },
            markLine: {
              silent: true,
              symbol: "none",
              data: [{ yAxis: 0 }],
              lineStyle: { color: "#94a3b8", type: "dotted", width: 1 },
              label: { show: false },
            },
          },
        ]),
      };
    }
    if (!xDomain) return null;
    return {
      animation: false,
      legend: {
        show: true,
        top: 0,
        data: ratioSeries.map((overlay) => overlay.name),
        textStyle: { color: "#64748b", fontSize: 12 },
      },
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: ratioYDomain[0],
        max: ratioYDomain[1],
        name: "log10(value / comparison)",
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: ratioSeries.flatMap((overlay) => [
        buildErrorBarSeries({
          name: `${overlay.name} ratio error`,
          data: overlay.ratioData.map((point) => [point[0], point[2], point[3]]),
          color: overlay.color,
        }),
        {
          type: "line",
          name: overlay.name,
          data: overlay.ratioData.map((point) => [point[0], point[1]]),
          showSymbol: false,
          connectNulls: false,
          lineStyle: { width: 1.35, color: overlay.color },
          itemStyle: { color: overlay.color },
          markLine: {
            silent: true,
            symbol: "none",
            data: [{ yAxis: 0 }],
            lineStyle: { color: "#94a3b8", type: "dotted", width: 1 },
            label: { show: false },
          },
        },
      ]),
    };
};
