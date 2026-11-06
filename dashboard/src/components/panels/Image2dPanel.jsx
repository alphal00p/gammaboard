import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Box, Button, Card, CardContent, MenuItem, TextField, Typography, useTheme } from "@mui/material";
import { PanelHeader, ControlBar } from "../common/PanelChrome";
import { asArray } from "../../utils/collections";
import { formatScientific } from "../../utils/formatters";
import { EmptyPlotPanel } from "./BasicPanels";
import FigureExportActions from "./FigureExportActions";
import HeatmapColorLimitControl from "./HeatmapColorLimitControl";
import ComplexPhaseLegend from "./ComplexPhaseLegend";
import { complexPhaseColorForValue } from "./complexColors";
import { FULL_ZOOM, normalizeZoomRange, readYZoomFromPanelValue, readZoomFromPanelValue, writeZoomPanelValue } from "./panelView";
import { buildScalarHeatmapScale, heatmapColorForValue, heatmapLegendGradient, isZeroCenteredScale,
  readHeatmapColorLimits, resetHeatmapColorLimits, writeHeatmapColorLimit } from "./heatmapColors";

const HEATMAP_LEGEND_WIDTH = 116;
const HEATMAP_LEGEND_GAP = 12;

const estimateHeatmapChartHeight = (width, height, panelWidth, margins) => {
  if (width <= 0 || height <= 0) return 360;
  const availableWidth =
    panelWidth > 0
      ? panelWidth
      : Math.min(920, Math.max(320, typeof window !== "undefined" ? window.innerWidth - 64 : 920));
  const legendWidth = HEATMAP_LEGEND_WIDTH + HEATMAP_LEGEND_GAP;
  const innerWidth = Math.max(1, availableWidth - legendWidth - margins.left - margins.right);
  const innerHeight = (innerWidth * height) / width;
  return Math.max(220, Math.round(innerHeight + margins.top + margins.bottom));
};

const parameterAtRasterIndex = (range, count, index) => {
  const [min, max] = asArray(range);
  if (!Number.isFinite(min) || !Number.isFinite(max) || count <= 0) return index;
  if (count === 1) return min;
  return min + ((max - min) * index) / (count - 1);
};

const rasterIndexRangeFromZoom = (count, zoomRange) => {
  const normalized = normalizeZoomRange(zoomRange) || FULL_ZOOM;
  const safeCount = Math.max(1, count);
  const start = Math.max(0, Math.min(safeCount - 1, Math.floor((normalized.start / 100) * safeCount)));
  const exclusiveEnd = Math.max(start + 1, Math.min(safeCount, Math.ceil((normalized.end / 100) * safeCount)));
  return { start, exclusiveEnd, count: exclusiveEnd - start };
};

const clampZoomRangeWithWidth = (start, width) => {
  const clampedWidth = Math.max(0.1, Math.min(100, width));
  const clampedStart = Math.max(0, Math.min(100 - clampedWidth, start));
  return { start: clampedStart, end: clampedStart + clampedWidth };
};

const zoomRangeAroundFraction = (zoomRange, fraction, scale) => {
  const normalized = normalizeZoomRange(zoomRange) || FULL_ZOOM;
  const currentWidth = Math.max(0.1, normalized.end - normalized.start);
  const nextWidth = Math.max(0.1, Math.min(100, currentWidth * scale));
  const anchor = normalized.start + currentWidth * Math.max(0, Math.min(1, fraction));
  return clampZoomRangeWithWidth(anchor - nextWidth * Math.max(0, Math.min(1, fraction)), nextWidth);
};

const panZoomRange = (zoomRange, deltaPercent) => {
  const normalized = normalizeZoomRange(zoomRange) || FULL_ZOOM;
  const width = Math.max(0.1, normalized.end - normalized.start);
  return clampZoomRangeWithWidth(normalized.start + deltaPercent, width);
};

const visiblePlotRect = (canvasWidth, canvasHeight, margins) => ({
  x: margins.left,
  y: margins.top,
  width: Math.max(1, canvasWidth - margins.left - margins.right),
  height: Math.max(1, canvasHeight - margins.top - margins.bottom),
});

const heatmapMetricLabel = (panelId, metricLabel = null) => {
  if (typeof metricLabel === "string" && metricLabel.trim()) return metricLabel.trim();
  if (typeof panelId !== "string") return "value";
  if (panelId.includes("oversampling")) return "PDF / |integrand| - 1";
  if (panelId.includes("log_pdf")) return "log10(normalized PDF)";
  if (panelId.includes("log_integrand")) return "log10(normalized integrand)";
  return "value";
};

const heatmapTooltipValueLines = (panelId, value, metricLabel = null, metricMode = null) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return ["value: n/a"];
  const label = heatmapMetricLabel(panelId, metricLabel);
  const lines = [`${label}: ${formatScientific(numeric, 6)}`];
  if (typeof panelId === "string" && panelId.includes("oversampling")) {
    const ratio = metricMode === "log10_ratio" ? 10 ** numeric : numeric + 1;
    if (Number.isFinite(ratio)) lines.push(`PDF / |integrand|: ${formatScientific(ratio, 6)}`);
  } else if (typeof panelId === "string" && panelId.startsWith("pdf_adaptation_")) {
    const factor = 10 ** numeric;
    if (Number.isFinite(factor)) lines.push(`linear factor: ${formatScientific(factor, 6)}`);
  }
  return lines;
};

const complexTooltipValueLines = (re, im, metricLabel = null) => {
  const real = Number(re);
  const imag = Number(im);
  if (!Number.isFinite(real) || !Number.isFinite(im)) return ["value: n/a"];
  const label = typeof metricLabel === "string" && metricLabel.trim() ? metricLabel.trim() : "complex value";
  return [
    `${label} re: ${formatScientific(real, 6)}`,
    `${label} im: ${formatScientific(imag, 6)}`,
    `|z|: ${formatScientific(Math.hypot(real, imag), 6)}`,
    `phase: ${formatScientific(Math.atan2(imag, real), 6)}`,
  ];
};

const HeatmapScaleLegend = ({ zmin, zmax, normalizationMode, panelId, metricLabel = null }) => {
  const max = Number(zmax);
  const min = Number(zmin);
  const showMidpoint = isZeroCenteredScale(normalizationMode) && min < 0 && max > 0;
  const label = heatmapMetricLabel(panelId, metricLabel);
  return (
    <Box sx={{ display: "grid", gap: 0.75 }}>
      <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.15 }}>
        {label}
      </Typography>
      <Box sx={{ display: "grid", gridTemplateColumns: "22px 1fr", gap: 1, alignItems: "stretch", height: 220 }}>
        <Box
          sx={{
            width: 22,
            height: "100%",
            borderRadius: 0.75,
            border: "1px solid", borderColor: "divider",
            background: heatmapLegendGradient(min, max, normalizationMode),
          }}
        />
        <Box sx={{ position: "relative", minWidth: 0 }}>
          <Typography
            variant="caption"
            sx={{ position: "absolute", top: -3, left: 0, color: "text.secondary", fontFamily: "monospace" }}
          >
            {formatScientific(max, 3)}
          </Typography>
          {showMidpoint ? (
            <Typography
              variant="caption"
              sx={{
                position: "absolute",
                top: "50%",
                left: 0,
                transform: "translateY(-50%)",
                color: "text.primary",
                fontFamily: "monospace",
                fontWeight: 700,
              }}
            >
              0
            </Typography>
          ) : null}
          <Typography
            variant="caption"
            sx={{ position: "absolute", bottom: -3, left: 0, color: "text.secondary", fontFamily: "monospace" }}
          >
            {formatScientific(min, 3)}
          </Typography>
        </Box>
      </Box>
    </Box>
  );
};

const ScalarImageHeatmapPanel = ({
  title,
  panelId = null,
  width,
  height,
  values,
  imagValues = null,
  invalidIndices,
  colorMode = "scalar_heatmap",
  normalizationMode,
  metricLabel = null,
  metricMode = null,
  xLabel = "t",
  yLabel = "s",
  xRange,
  yRange,
  value = undefined,
  onValueChange = null,
}) => {
  const figureRef = useRef(null);
  const canvasRef = useRef(null);
  const scratchCanvasRef = useRef(null);
  const { palette } = useTheme();
  const dragRef = useRef(null);
  const [panelWidth, setPanelWidth] = useState(0);
  const [tooltip, setTooltip] = useState(null);
  const [colorResetVersion, setColorResetVersion] = useState(0);
  const totalCells = Math.max(0, width * height);
  const boundedValues = useMemo(() => values.slice(0, totalCells), [totalCells, values]);
  const boundedImagValues = useMemo(() => (imagValues ? imagValues.slice(0, totalCells) : null), [imagValues, totalCells]);
  const supportsColorScale = ["zero_centered", "linear"].includes(normalizationMode) && colorMode !== "complex_phase";
  const supportsColorLimits = supportsColorScale && panelId && typeof onValueChange === "function";
  const colorScale = supportsColorScale && ["zero_centered", "linear"].includes(value?.colorScale)
    ? value.colorScale : normalizationMode;
  const colorLimits = readHeatmapColorLimits(value, metricMode, colorScale);
  const complexMaxMagnitude = useMemo(() => {
    if (colorMode !== "complex_phase" || !boundedImagValues) return 0;
    return boundedValues.reduce((max, re, index) => {
      if (invalidIndices?.has(index)) return max;
      const im = boundedImagValues[index];
      if (!Number.isFinite(re) || !Number.isFinite(im)) return max;
      return Math.max(max, Math.hypot(re, im));
    }, 0);
  }, [boundedImagValues, boundedValues, colorMode, invalidIndices]);
  const defaultScale = useMemo(
    () => buildScalarHeatmapScale(boundedValues, colorScale, null, invalidIndices),
    [boundedValues, invalidIndices, colorScale],
  );
  const { zmin, zmax } = useMemo(
    () => buildScalarHeatmapScale(boundedValues, colorScale, colorLimits, invalidIndices),
    [boundedValues, colorLimits, invalidIndices, colorScale],
  );
  const heatmapMargins = useMemo(() => ({ left: 56, right: 24, top: 16, bottom: 44 }), []);
  const zoomRange = useMemo(() => readZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const yZoomRange = useMemo(() => readYZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const chartHeight = useMemo(() => {
    return estimateHeatmapChartHeight(width, height, panelWidth, heatmapMargins);
  }, [heatmapMargins.bottom, heatmapMargins.left, heatmapMargins.right, heatmapMargins.top, height, panelWidth, width]);

  const updateZoom = useCallback(
    (nextX, nextY) => {
      if (typeof onValueChange !== "function" || !panelId) return;
      onValueChange(panelId, writeZoomPanelValue(value, nextX, null, nextY), false);
    },
    [onValueChange, panelId, value],
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || panelWidth <= 0 || chartHeight <= 0) return undefined;
    const ctx = canvas.getContext("2d");
    if (!ctx) return undefined;
    const dpr = Math.max(1, window.devicePixelRatio || 1);
    const cssWidth = Math.max(1, Math.floor(panelWidth - HEATMAP_LEGEND_WIDTH - HEATMAP_LEGEND_GAP));
    const cssHeight = Math.max(1, Math.floor(chartHeight));
    const rafId = requestAnimationFrame(() => {
      canvas.width = Math.max(1, Math.floor(cssWidth * dpr));
      canvas.height = Math.max(1, Math.floor(cssHeight * dpr));
      canvas.style.width = `${cssWidth}px`;
      canvas.style.height = `${cssHeight}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, cssWidth, cssHeight);
      ctx.fillStyle = palette.background.paper;
      ctx.fillRect(0, 0, cssWidth, cssHeight);

      const plot = visiblePlotRect(cssWidth, cssHeight, heatmapMargins);
      const xWindow = rasterIndexRangeFromZoom(width, zoomRange);
      const yWindow = rasterIndexRangeFromZoom(height, yZoomRange);
      const visibleCols = Math.max(1, xWindow.count);
      const visibleRows = Math.max(1, yWindow.count);
      const scratch = scratchCanvasRef.current || document.createElement("canvas");
      scratchCanvasRef.current = scratch;
      scratch.width = visibleCols;
      scratch.height = visibleRows;
      const scratchCtx = scratch.getContext("2d");
      const image = scratchCtx.createImageData(visibleCols, visibleRows);
      for (let row = 0; row < visibleRows; row += 1) {
        const sourceRow = yWindow.start + row;
        for (let col = 0; col < visibleCols; col += 1) {
          const sourceCol = xWindow.start + col;
          const sourceIndex = sourceRow * width + sourceCol;
          const targetIndex = (row * visibleCols + col) * 4;
          const rgb = invalidIndices?.has(sourceIndex)
            ? [255, 0, 255]
            : colorMode === "complex_phase" && boundedImagValues
              ? complexPhaseColorForValue(boundedValues[sourceIndex], boundedImagValues[sourceIndex], complexMaxMagnitude)
              : heatmapColorForValue(boundedValues[sourceIndex], zmin, zmax, colorScale);
          image.data[targetIndex] = rgb[0];
          image.data[targetIndex + 1] = rgb[1];
          image.data[targetIndex + 2] = rgb[2];
          image.data[targetIndex + 3] = 255;
        }
      }
      scratchCtx.putImageData(image, 0, 0);
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(scratch, plot.x, plot.y, plot.width, plot.height);

      ctx.strokeStyle = palette.text.secondary;
      ctx.lineWidth = 1;
      ctx.strokeRect(plot.x, plot.y, plot.width, plot.height);
      ctx.fillStyle = palette.text.secondary;
      ctx.font = "11px sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "top";
      const xTickCount = 6;
      for (let tick = 0; tick <= xTickCount; tick += 1) {
        const fraction = tick / xTickCount;
        const x = plot.x + plot.width * fraction;
        const index = xWindow.start + (visibleCols - 1) * fraction;
        ctx.beginPath();
        ctx.moveTo(x, plot.y + plot.height);
        ctx.lineTo(x, plot.y + plot.height + 4);
        ctx.stroke();
        ctx.fillText(formatScientific(parameterAtRasterIndex(xRange, width, index), 2), x, plot.y + plot.height + 7);
      }
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      const yTickCount = 6;
      for (let tick = 0; tick <= yTickCount; tick += 1) {
        const fraction = tick / yTickCount;
        const y = plot.y + plot.height * fraction;
        const index = yWindow.start + (visibleRows - 1) * fraction;
        ctx.beginPath();
        ctx.moveTo(plot.x - 4, y);
        ctx.lineTo(plot.x, y);
        ctx.stroke();
        ctx.fillText(formatScientific(parameterAtRasterIndex(yRange, height, index), 2), plot.x - 7, y);
      }
      ctx.textAlign = "center";
      ctx.textBaseline = "alphabetic";
      ctx.fillText(xLabel || "x", plot.x + plot.width / 2, cssHeight - 7);
      ctx.save();
      ctx.translate(12, plot.y + plot.height / 2);
      ctx.rotate(-Math.PI / 2);
      ctx.fillText(yLabel || "y", 0, 0);
      ctx.restore();
    });
    return () => {
      cancelAnimationFrame(rafId);
    };
  }, [
    boundedValues,
    boundedImagValues,
    chartHeight,
    colorMode,
    complexMaxMagnitude,
    palette,
    heatmapMargins,
    height,
    invalidIndices,
    colorScale,
    panelWidth,
    width,
    xRange,
    xLabel,
    yRange,
    yLabel,
    yZoomRange,
    zmax,
    zmin,
    zoomRange,
  ]);
  useEffect(() => {
    const element = figureRef.current;
    if (!element || typeof ResizeObserver === "undefined") return undefined;
    const updateWidth = () => {
      const measured = Math.max(0, Math.floor(element.getBoundingClientRect().width));
      setPanelWidth((current) => (current !== measured ? measured : current));
    };
    updateWidth();
    const rafId = requestAnimationFrame(updateWidth);
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const nextWidth = Math.max(0, Math.floor(entry.contentRect.width));
      setPanelWidth((current) => (current !== nextWidth ? nextWidth : current));
    });
    observer.observe(element);
    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
    };
  }, []);
  useEffect(() => {
    const element = figureRef.current;
    if (!element) return undefined;
    const preventPageScroll = (event) => {
      event.preventDefault();
    };
    element.addEventListener("wheel", preventPageScroll, { passive: false });
    return () => {
      element.removeEventListener("wheel", preventPageScroll);
    };
  }, []);

  const canvasPointFromEvent = useCallback((event) => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    return {
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      width: rect.width,
      height: rect.height,
    };
  }, []);

  const tooltipFromPoint = useCallback(
    (point) => {
      if (!point) return null;
      const plot = visiblePlotRect(point.width, point.height, heatmapMargins);
      if (
        point.x < plot.x ||
        point.x > plot.x + plot.width ||
        point.y < plot.y ||
        point.y > plot.y + plot.height
      ) {
        return null;
      }
      const xWindow = rasterIndexRangeFromZoom(width, zoomRange);
      const yWindow = rasterIndexRangeFromZoom(height, yZoomRange);
      const col = Math.max(
        xWindow.start,
        Math.min(
          xWindow.exclusiveEnd - 1,
          xWindow.start + Math.floor(((point.x - plot.x) / plot.width) * xWindow.count),
        ),
      );
      const row = Math.max(
        yWindow.start,
        Math.min(
          yWindow.exclusiveEnd - 1,
          yWindow.start + Math.floor(((point.y - plot.y) / plot.height) * yWindow.count),
        ),
      );
      const index = row * width + col;
      const lines = invalidIndices?.has(index)
        ? ["invalid value"]
        : [
            `${xLabel || "x"}: ${formatScientific(parameterAtRasterIndex(xRange, width, col), 4)}`,
            `${yLabel || "y"}: ${formatScientific(parameterAtRasterIndex(yRange, height, row), 4)}`,
            ...(colorMode === "complex_phase" && boundedImagValues
              ? complexTooltipValueLines(boundedValues[index], boundedImagValues[index], metricLabel)
              : heatmapTooltipValueLines(panelId, boundedValues[index], metricLabel, metricMode)),
          ];
      return {
        x: Math.min(point.width - 180, point.x + 12),
        y: Math.max(8, point.y - 8),
        lines,
      };
    },
    [
      boundedValues,
      boundedImagValues,
      colorMode,
      heatmapMargins,
      height,
      invalidIndices,
      metricLabel,
      metricMode,
      panelId,
      width,
      xRange,
      xLabel,
      yRange,
      yLabel,
      yZoomRange,
      zoomRange,
    ],
  );

  const handleWheel = useCallback(
    (event) => {
      const point = canvasPointFromEvent(event);
      if (!point) return;
      event.preventDefault();
      const plot = visiblePlotRect(point.width, point.height, heatmapMargins);
      if (
        point.x < plot.x ||
        point.x > plot.x + plot.width ||
        point.y < plot.y ||
        point.y > plot.y + plot.height
      ) {
        return;
      }
      const scale = event.deltaY < 0 ? 0.82 : 1.22;
      updateZoom(
        zoomRangeAroundFraction(zoomRange, (point.x - plot.x) / plot.width, scale),
        zoomRangeAroundFraction(yZoomRange, (point.y - plot.y) / plot.height, scale),
      );
    },
    [canvasPointFromEvent, heatmapMargins, updateZoom, yZoomRange, zoomRange],
  );

  const handlePointerMove = useCallback(
    (event) => {
      const point = canvasPointFromEvent(event);
      if (!point) return;
      if (dragRef.current) {
        const drag = dragRef.current;
        const plot = visiblePlotRect(point.width, point.height, heatmapMargins);
        const xSpan = Math.max(0.1, drag.zoom.end - drag.zoom.start);
        const ySpan = Math.max(0.1, drag.yZoom.end - drag.yZoom.start);
        updateZoom(
          panZoomRange(drag.zoom, -((point.x - drag.x) / plot.width) * xSpan),
          panZoomRange(drag.yZoom, -((point.y - drag.y) / plot.height) * ySpan),
        );
        return;
      }
      setTooltip(tooltipFromPoint(point));
    },
    [canvasPointFromEvent, heatmapMargins, tooltipFromPoint, updateZoom],
  );

  const handlePointerDown = useCallback(
    (event) => {
      const point = canvasPointFromEvent(event);
      if (!point) return;
      const plot = visiblePlotRect(point.width, point.height, heatmapMargins);
      if (
        point.x < plot.x ||
        point.x > plot.x + plot.width ||
        point.y < plot.y ||
        point.y > plot.y + plot.height
      ) {
        return;
      }
      event.currentTarget.setPointerCapture?.(event.pointerId);
      dragRef.current = {
        x: point.x,
        y: point.y,
        zoom: normalizeZoomRange(zoomRange) || FULL_ZOOM,
        yZoom: normalizeZoomRange(yZoomRange) || FULL_ZOOM,
      };
      setTooltip(null);
    },
    [canvasPointFromEvent, heatmapMargins, yZoomRange, zoomRange],
  );

  const handlePointerUp = useCallback((event) => {
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    dragRef.current = null;
  }, []);

  return (
    <Card variant="outlined">
      <CardContent>
        <PanelHeader title={title}>
          <FigureExportActions
            imageFormat="png"
            baseName={panelId || title || "image2d"}
            payload={{
              panel_id: panelId,
              kind: "image2d",
              state: {
                width,
                height,
                x_range: xRange,
                y_range: yRange,
                values,
                imag_values: imagValues,
                invalid_indices: Array.from(invalidIndices || []),
                normalization_mode: colorScale,
                color_mode: colorMode,
                ...(colorMode === "complex_phase"
                  ? { phase_range_radians: [-Math.PI, Math.PI], magnitude_range: [0, complexMaxMagnitude] }
                  : { color_range: [zmin, zmax] }),
                metric_label: metricLabel,
                metric_mode: metricMode,
              },
            }}
            elementRef={figureRef}
            onResetView={
              panelId && typeof onValueChange === "function"
                ? () => onValueChange(panelId, writeZoomPanelValue(value, FULL_ZOOM, null, FULL_ZOOM), false)
                : null
            }
          />
        </PanelHeader>
        {supportsColorLimits ? (
          <ControlBar sx={{ mb: 2 }}>
            <TextField
              select
              label="Color scale"
              size="small"
              value={colorScale}
              slotProps={{ select: { inputProps: { "aria-label": "Color scale" } } }}
              onChange={(event) => onValueChange(panelId, {
                ...resetHeatmapColorLimits(value), colorScale: event.target.value,
              }, false)}
              sx={{ minWidth: 145 }}
            >
              <MenuItem value="linear">linear</MenuItem>
              <MenuItem value="zero_centered">zero-origin</MenuItem>
            </TextField>
            <HeatmapColorLimitControl
              key={`min:${metricMode}:${colorScale}:${colorResetVersion}`}
              side="min"
              limit={zmin}
              otherLimit={zmax}
              defaultScale={defaultScale}
              zeroOrigin={colorScale === "zero_centered"}
              onChange={(next) => onValueChange(panelId, writeHeatmapColorLimit(value, metricMode, "min", next, colorScale), false)}
            />
            <HeatmapColorLimitControl
              key={`max:${metricMode}:${colorScale}:${colorResetVersion}`}
              side="max"
              limit={zmax}
              otherLimit={zmin}
              defaultScale={defaultScale}
              zeroOrigin={colorScale === "zero_centered"}
              onChange={(next) => onValueChange(panelId, writeHeatmapColorLimit(value, metricMode, "max", next, colorScale), false)}
            />
            <Button
              size="small"
              onClick={() => {
                setColorResetVersion((version) => version + 1);
                onValueChange(panelId, resetHeatmapColorLimits(value), false);
              }}
            >
              Auto limits
            </Button>
          </ControlBar>
        ) : null}
        <Box
          ref={figureRef}
          sx={{
            width: "min(100%, 920px)",
            mx: "auto",
            height: `${chartHeight}px`,
          }}
        >
          <Box sx={{ display: "flex", alignItems: "center", gap: `${HEATMAP_LEGEND_GAP}px`, height: "100%" }}>
            <Box sx={{ flex: 1, minWidth: 0, height: "100%", position: "relative" }}>
              <canvas
                ref={canvasRef}
                onWheel={handleWheel}
                onPointerMove={handlePointerMove}
                onPointerDown={handlePointerDown}
                onPointerUp={handlePointerUp}
                onPointerCancel={handlePointerUp}
                onPointerLeave={() => {
                  if (!dragRef.current) setTooltip(null);
                }}
                onDoubleClick={() => updateZoom(FULL_ZOOM, FULL_ZOOM)}
                style={{
                  display: "block",
                  width: "100%",
                  height: "100%",
                  cursor: dragRef.current ? "grabbing" : "grab",
                  touchAction: "none",
                }}
              />
              {tooltip ? (
                <Box
                  sx={{
                    position: "absolute",
                    left: tooltip.x,
                    top: tooltip.y,
                    bgcolor: "rgba(15,23,42,0.92)",
                    color: "#fff",
                    borderRadius: 1,
                    px: 1,
                    py: 0.75,
                    pointerEvents: "none",
                    fontSize: 12,
                    fontFamily: "monospace",
                    lineHeight: 1.35,
                    maxWidth: 260,
                    zIndex: 1,
                  }}
                >
                  {tooltip.lines.map((line) => (
                    <Box key={line}>{line}</Box>
                  ))}
                </Box>
              ) : null}
            </Box>
            <Box sx={{ width: HEATMAP_LEGEND_WIDTH, flexShrink: 0 }}>
              {colorMode === "complex_phase" ? <ComplexPhaseLegend maxMagnitude={complexMaxMagnitude} /> : <HeatmapScaleLegend
                zmin={zmin}
                zmax={zmax}
                normalizationMode={colorScale}
                panelId={panelId}
                metricLabel={metricLabel}
              />}
            </Box>
          </Box>
        </Box>
      </CardContent>
    </Card>
  );
};


const imagePanelPropsEqual = (left, right) =>
  left.title === right.title &&
  left.value === right.value &&
  left.onValueChange === right.onValueChange &&
  left.state?.panel_id === right.state?.panel_id &&
  left.state?.width === right.state?.width &&
  left.state?.height === right.state?.height &&
  left.state?.color_mode === right.state?.color_mode &&
  left.state?.normalization_mode === right.state?.normalization_mode &&
  left.state?.metric_label === right.state?.metric_label &&
  left.state?.metric_mode === right.state?.metric_mode &&
  left.state?.x_label === right.state?.x_label &&
  left.state?.y_label === right.state?.y_label &&
  left.state?.x_range === right.state?.x_range &&
  left.state?.y_range === right.state?.y_range &&
  left.state?.values === right.state?.values &&
  left.state?.imag_values === right.state?.imag_values &&
  left.state?.invalid_indices === right.state?.invalid_indices;

const Image2dPanel = memo(({ title, state, value = undefined, onValueChange = null }) => {
  const width = Number(state?.width) || 0;
  const height = Number(state?.height) || 0;
  const values = useMemo(() => asArray(state?.values), [state?.values]);
  const imagValues = useMemo(() => {
    const next = asArray(state?.imag_values);
    return next.length > 0 ? next : null;
  }, [state?.imag_values]);
  const invalidIndices = useMemo(() => new Set(asArray(state?.invalid_indices)), [state?.invalid_indices]);
  const colorMode = state?.color_mode || "scalar_heatmap";
  // Older servers used symmetric/zero_centered for every PDF heatmap.
  // Apply the new defaults to their payloads too, leaving other images alone.
  const normalizationMode = ["symmetric", "zero_centered"].includes(state?.normalization_mode) && state?.panel_id?.startsWith("pdf_adaptation_")
    ? (["pdf_adaptation_log_integrand", "pdf_adaptation_log_pdf"].includes(state.panel_id) ? "linear" : "zero_centered")
    : state?.normalization_mode || "min_max";
  const metricLabel = typeof state?.metric_label === "string" ? state.metric_label : null;
  const metricMode = typeof state?.metric_mode === "string" ? state.metric_mode : null;
  const xLabel = typeof state?.x_label === "string" ? state.x_label : "t";
  const yLabel = typeof state?.y_label === "string" ? state.y_label : "s";
  const xRange = useMemo(() => asArray(state?.x_range), [state?.x_range]);
  const yRange = useMemo(() => asArray(state?.y_range), [state?.y_range]);
  const scalarValues = useMemo(() => {
    if (!imagValues || colorMode === "complex_phase") return values;
    return values.map((re, index) => {
      const im = imagValues[index] || 0;
      if (!Number.isFinite(re) || !Number.isFinite(im)) return Number.NaN;
      return Math.hypot(re, im);
    });
  }, [colorMode, imagValues, values]);
  if (width <= 0 || height <= 0 || values.length === 0 || invalidIndices.size >= values.length) return <EmptyPlotPanel title={title} />;
  return (
    <ScalarImageHeatmapPanel
      title={title}
      panelId={state?.panel_id ?? null}
      width={width}
      height={height}
      values={scalarValues}
      imagValues={colorMode === "complex_phase" ? imagValues : null}
      invalidIndices={invalidIndices}
      colorMode={colorMode}
      normalizationMode={normalizationMode}
      metricLabel={metricLabel}
      metricMode={metricMode}
      xLabel={xLabel}
      yLabel={yLabel}
      xRange={xRange}
      yRange={yRange}
      value={value}
      onValueChange={onValueChange}
    />
  );
}, imagePanelPropsEqual);
Image2dPanel.displayName = "Image2dPanel";

export default Image2dPanel;
