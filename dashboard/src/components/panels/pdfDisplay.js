import {
  diagnosticHistograms,
  pdfComparison,
  sharedColorLimits,
} from "./pdfDiagnostics";
import { lineComparisonOption, planeComparisonOption } from "./pdfPlotOptions";
import { finite } from "./strataUtils";
import { formatAxisValue } from "./chartPrimitives";

export const DENSITY_MAPS = {
  Viridis: ["#440154", "#3b528b", "#21918c", "#5ec962", "#fde725"],
  Cividis: ["#00224e", "#434e6c", "#7d7c78", "#bcae6c", "#fee838"],
  Inferno: ["#000004", "#57106e", "#bc3754", "#f98e09", "#fcffa4"],
};
export const SIGN_COLORS = ["#d55e00", "#ffffff", "#0072b2"];
export const MISSING_COLOR = "#888888";
export const ZERO_COLOR = "#e5e5e5";
export const MATCH_COLOR = "#c6e8c3";
const interpolate = (colors, fraction) => {
  const x = Math.max(0, Math.min(1, fraction)) * (colors.length - 1),
    i = Math.min(colors.length - 2, Math.floor(x));
  const channels = [1, 3, 5].map((offset) => {
    const a = parseInt(colors[i].slice(offset, offset + 2), 16),
      b = parseInt(colors[i + 1].slice(offset, offset + 2), 16);
    return Math.round(a + (b - a) * (x - i))
      .toString(16)
      .padStart(2, "0");
  });
  return `#${channels.join("")}`;
};
export const mismatchColor = (v, limits) =>
  v < 0
    ? interpolate(
        [MATCH_COLOR, "#c82d3c"],
        v / Math.min(limits.min, -Number.MIN_VALUE),
      )
    : interpolate(
        [MATCH_COLOR, "#2466c5"],
        v / Math.max(limits.max, Number.MIN_VALUE),
      );
export const densityColor = (v, limits, log, palette) => {
  if (!finite(v)) return MISSING_COLOR;
  if (log && v <= 0) return ZERO_COLOR;
  const x = log ? Math.log10(v) : v;
  return interpolate(
    DENSITY_MAPS[palette] || DENSITY_MAPS.Viridis,
    (x - limits.min) / (limits.max - limits.min),
  );
};
export const mismatchValue = (c, i, metric) => {
  if (!finite(c.pdf[i]) || !finite(c.target[i])) return null;
  if (c.status[i] === "uncovered")
    return metric === "relative" ? -1 : -Infinity;
  if (c.status[i] === "leakage") return Infinity;
  return metric === "relative"
    ? (c.relative[i] ?? (c.pdf[i] > 0 && c.target[i] > 0 ? Infinity : null))
    : c.logRatio[i];
};
export const plotColor = (model, i) => {
  if (model.kind === "sign")
    return finite(model.values[i])
      ? SIGN_COLORS[model.values[i] + 1]
      : MISSING_COLOR;
  if (model.kind !== "mismatch")
    return densityColor(
      model.values[i],
      model.limits,
      model.log,
      model.palette,
    );
  const v = mismatchValue(model.c, i, model.metric);
  if (model.c.status[i] === "both_zero") return ZERO_COLOR;
  return v == null ? MISSING_COLOR : mismatchColor(v, model.limits);
};
export const colorGradient = (model) => {
  const stops = Array.from({ length: 65 }, (_, i) => {
    const t = i / 64,
      v = model.limits.min + t * (model.limits.max - model.limits.min);
    const color =
      model.kind === "mismatch"
        ? mismatchColor(v, model.limits)
        : densityColor(
            model.log ? 10 ** v : v,
            model.limits,
            model.log,
            model.palette,
          );
    return `${color} ${100 * t}%`;
  });
  return `linear-gradient(to right, ${stops.join(",")})`;
};
export const comparisonModels = (state, scope, settings, comparison) => {
  const c = comparison || pdfComparison(state, scope, settings.normalization);
  const log = settings.log,
    palette = settings.palette || "Viridis",
    metric = settings.metric || "log_ratio";
  const limits = (role, arrays, logarithmic = false) =>
    sharedColorLimits(arrays, {
      log: logarithmic,
      manual: settings.limits?.[role] && {
        min: logarithmic
          ? Math.log10(settings.limits[role].min)
          : settings.limits[role].min,
        max: logarithmic
          ? Math.log10(settings.limits[role].max)
          : settings.limits[role].max,
      },
    });
  const shared = limits("density", [c.target, c.pdf], log);
  const density = (role, values) => ({
    kind: role,
    c,
    values,
    log,
    palette,
    limits: settings.linked !== false ? shared : limits(role, [values], log),
  });
  return {
    c,
    target: density("target", c.target),
    sampler: density("sampler", c.pdf),
    mismatch: {
      kind: "mismatch",
      c,
      metric,
      values: metric === "relative" ? c.relative : c.logRatio,
      limits: limits("mismatch", [
        metric === "relative" ? c.relative : c.logRatio,
        [0],
      ]),
    },
    sign: { kind: "sign", c, values: c.sign },
  };
};
export const planeOption = (model, value = {}, raster = null) => {
  const { c } = model;
  const sign = model.kind === "sign";
  const option = planeComparisonOption(c, raster ? [] : model.values, {
    label: model.kind,
    limits: model.limits,
    log: model.log,
    categories: sign
      ? SIGN_COLORS.map((color, i) => ({ color, label: ["−", "0", "+"][i] }))
      : null,
    value,
  });
  option.visualMap = undefined; // A keyboard-accessible colorbar belongs to the figure header.
  option.grid = { left: 64, right: 24, top: 20, bottom: 64 };
  if (raster) {
    option.series = [
      {
        type: "custom",
        data: [0],
        silent: true,
        renderItem: (_params, api) => {
          const [x0, y0] = api.coord([0, 0]),
            [dx, dy] = api.size([1, 1]);
          return {
            type: "group",
            clipPath: {
              type: "rect",
              shape: {
                x: _params.coordSys.x,
                y: _params.coordSys.y,
                width: _params.coordSys.width,
                height: _params.coordSys.height,
              },
            },
            children: [
              {
                type: "image",
                style: {
                  image: raster,
                  x: x0 - Math.abs(dx) / 2,
                  y: y0 + Math.abs(dy) / 2 - Math.abs(dy) * c.axes.y.length,
                  width: Math.abs(dx) * c.axes.x.length,
                  height: Math.abs(dy) * c.axes.y.length,
                },
              },
            ],
          };
        },
      },
    ];
    option.tooltip = { show: false };
  } else {
    // Exports retain actual vector cells, including zeros and missing values.
    option.series[0].data = model.values.map((v, i) => ({
      value: [
        i % c.axes.x.length,
        Math.floor(i / c.axes.x.length),
        v,
        v,
        c.actual[i],
      ],
      itemStyle: { color: plotColor(model, i) },
    }));
  }
  return option;
};
export const curveOption = (
  models,
  settings,
  auxiliary = false,
  value = {},
) => {
  const kind = auxiliary ? settings.auxiliary : "density";
  const option = lineComparisonOption(models.c, {
    view: kind === "sign" ? "mismatch" : kind,
    metric: settings.metric,
    mode: settings.normalization === "slice" ? "shape" : "reference",
    log: !auxiliary && settings.log,
    prefixes: [],
    value,
  });
  if (kind === "sign") {
    option.series = [
      {
        name: "Projection sign",
        type: "line",
        step: "middle",
        showSymbol: false,
        data: models.c.sign.map((v, i) => [models.c.axes.x[i], v]),
      },
    ];
    option.yAxis = {
      type: "value",
      min: -1,
      max: 1,
      interval: 1,
      axisLabel: { formatter: (v) => (v > 0 ? "+" : v < 0 ? "−" : "0") },
    };
  } else if (!auxiliary)
    option.yAxis[0].name =
      settings.normalization === "slice" ? "Relative density" : "Density";
  return option;
};
export const distributionOption = (models, settings, mismatch = false) => {
  const arrays = mismatch
    ? [models.mismatch.values]
    : [models.c.target, models.c.pdf];
  const log = !mismatch && Boolean(settings.histogramLog);
  const bins = diagnosticHistograms(
    arrays,
    40,
    log,
    settings.histogramWeighted !== false ? models.c.joint : null,
  );
  return {
    animation: false,
    legend: { top: 0 },
    grid: { left: 70, right: 24, top: 40, bottom: 65 },
    tooltip: { trigger: "axis" },
    xAxis: {
      type: log ? "log" : "value",
      name: mismatch
        ? settings.metric === "relative"
          ? "PDF / target − 1"
          : "log₁₀(PDF / target)"
        : "Value",
      nameLocation: "middle",
      nameGap: 32,
      axisLabel: { formatter: formatAxisValue },
    },
    yAxis: {
      type: "value",
      name: "Density",
      axisLine: { onZero: false },
      axisLabel: { formatter: formatAxisValue },
    },
    dataZoom: [
      { type: "inside", zoomOnMouseWheel: "ctrl" },
      { type: "slider", height: 16, bottom: 4 },
    ],
    series: bins.map((b, i) => ({
      name: mismatch ? "Mismatch" : ["Target", "Sampler"][i],
      type: "line",
      showSymbol: false,
      itemStyle: { color: ["#009e73", "#0072b2"][i] },
      lineStyle: {
        color: ["#009e73", "#0072b2"][i],
        type: i ? "dashed" : "solid",
      },
      data: b.flatMap((bin) => [
        [bin.start, bin.value],
        [bin.stop, bin.value],
      ]),
    })),
  };
};
export const rasterImage = (model) => {
  const width = model.c.axes.x.length,
    height = model.c.axes.y.length;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d"),
    pixels = context.createImageData(width, height);
  for (let i = 0; i < model.values.length; i++) {
    const color = plotColor(model, i),
      offset = ((height - 1 - Math.floor(i / width)) * width + (i % width)) * 4;
    for (let channel = 0; channel < 3; channel++)
      pixels.data[offset + channel] = parseInt(
        color.slice(1 + 2 * channel, 3 + 2 * channel),
        16,
      );
    pixels.data[offset + 3] = 255;
  }
  context.putImageData(pixels, 0, 0);
  return canvas;
};
