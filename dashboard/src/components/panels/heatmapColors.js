import { viridisColors } from "./viridisColors";

export const scalarHeatmapColors = ["#1d4ed8", "#16a34a", "#dc2626"];
const hexToRgb = (hex) => [1, 3, 5].map((start) => Number.parseInt(hex.slice(start, start + 2), 16));
const scalarHeatmapRgb = scalarHeatmapColors.map(hexToRgb);
const viridisRgb = viridisColors.map(hexToRgb);
const viridisGradient = `linear-gradient(to top, ${viridisColors.join(", ")})`;

export const isZeroCenteredScale = (mode) => mode === "zero_centered" || mode === "symmetric";

export const buildScalarHeatmapScale = (values, mode, limits = null, invalidIndices = null) => {
  let min = Infinity;
  let max = -Infinity;
  // Avoid spreading large images into Math.min/Math.max, and exclude masked
  // placeholder values as well as nonfinite pixels from automatic limits.
  values.forEach((value, index) => {
    if (!Number.isFinite(value) || invalidIndices?.has(index)) return;
    min = Math.min(min, value);
    max = Math.max(max, value);
  });
  if (mode === "zero_centered") {
    const defaults = !Number.isFinite(min) || (min === 0 && max === 0)
      ? { zmin: -1, zmax: 1 }
      : { zmin: Math.min(0, min), zmax: Math.max(0, max) };
    const zmin = Number.isFinite(limits?.min) && limits.min <= 0 ? limits.min : defaults.zmin;
    const zmax = Number.isFinite(limits?.max) && limits.max >= 0 ? limits.max : defaults.zmax;
    return zmin < zmax ? { zmin, zmax } : defaults;
  }
  if (!Number.isFinite(min)) {
    if (mode !== "linear") return { zmin: 0, zmax: 1 };
    min = 0;
    max = 1;
  }
  if (mode === "symmetric") {
    const extent = Math.max(Math.abs(min), Math.abs(max), 1e-12);
    return { zmin: -extent, zmax: extent };
  }
  if (min === max) {
    const padding = Math.abs(min) > 0 ? Math.abs(min) * 0.1 : 1;
    min -= padding;
    max += padding;
  }
  const defaults = { zmin: min, zmax: max };
  if (mode !== "linear") return defaults;
  const zmin = Number.isFinite(limits?.min) ? limits.min : min;
  const zmax = Number.isFinite(limits?.max) ? limits.max : max;
  return zmin < zmax ? { zmin, zmax } : defaults;
};

const mixRgb = (left, right, t) => {
  const clamped = Math.max(0, Math.min(1, t));
  return left.map((channel, index) => Math.round(channel + (right[index] - channel) * clamped));
};

export const heatmapColorForValue = (value, zmin, zmax, mode = "min_max") => {
  if (!Number.isFinite(value)) return [255, 0, 255];
  const [negative, neutral, positive] = scalarHeatmapRgb;
  if (isZeroCenteredScale(mode)) {
    const clamped = Math.max(zmin, Math.min(zmax, value));
    if (clamped === 0) return neutral;
    // Each sign gets its own slope, so asymmetric limits never move zero's color.
    if (clamped < 0) return mixRgb(neutral, negative, clamped / zmin);
    return mixRgb(neutral, positive, clamped / zmax);
  }
  const ratio = zmax > zmin ? Math.max(0, Math.min(1, (value - zmin) / (zmax - zmin))) : 0.5;
  if (mode === "linear") {
    const position = ratio * (viridisRgb.length - 1);
    const index = Math.floor(position);
    return mixRgb(viridisRgb[index], viridisRgb[Math.min(index + 1, viridisRgb.length - 1)], position - index);
  }
  if (ratio <= 0.5) return mixRgb(negative, neutral, ratio * 2);
  return mixRgb(neutral, positive, (ratio - 0.5) * 2);
};

export const heatmapLegendGradient = (zmin, zmax, mode) => {
  if (mode === "linear") return viridisGradient;
  const [negative, neutral, positive] = scalarHeatmapColors;
  if (isZeroCenteredScale(mode)) {
    if (zmin === 0) return `linear-gradient(to top, ${neutral}, ${positive})`;
    if (zmax === 0) return `linear-gradient(to top, ${negative}, ${neutral})`;
  }
  return `linear-gradient(to top, ${negative} 0%, ${neutral} 50%, ${positive} 100%)`;
};

// Metric switches change units, and zero-origin limits often pin an endpoint
// to zero. Neither should leak into another metric or a linear color scale.
export const readHeatmapColorLimits = (value, metricMode, colorScale = "zero_centered") => {
  const limits = value?.colorLimits;
  return limits && limits.metricMode === metricMode && (limits?.colorScale || "zero_centered") === colorScale ? limits : null;
};

export const writeHeatmapColorLimit = (value, metricMode, side, limit, colorScale = "zero_centered") => ({
  ...value,
  colorLimits: { ...readHeatmapColorLimits(value, metricMode, colorScale), metricMode, colorScale, [side]: limit },
});

export const resetHeatmapColorLimits = (value) => {
  const next = { ...value };
  delete next.colorLimits;
  delete next.spread;
  return next;
};
