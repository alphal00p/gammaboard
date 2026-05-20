import { asArray } from "../../utils/collections";
import { formatScientific } from "../../utils/formatters";
import { isObject } from "./panelView";

export const finiteNumberOrNull = (value) =>
  typeof value === "number" && Number.isFinite(value) ? value : null;

export const metricNumber = (bin, metricName) => {
  if (typeof metricName !== "string" || metricName.length === 0) return null;
  if (metricName === "error") return finiteNumberOrNull(bin?.error);
  const topLevel = finiteNumberOrNull(bin?.[metricName]);
  if (topLevel != null) return topLevel;
  return finiteNumberOrNull(bin?.metrics?.[metricName]?.value);
};

export const metricError = (bin, metricName) => {
  if (metricName === "value") return finiteNumberOrNull(bin?.error);
  return finiteNumberOrNull(bin?.metrics?.[metricName]?.error);
};

export const normalizeHistogramViews = (views) =>
  asArray(views)
    .filter((view) => isObject(view) && typeof view.id === "string" && typeof view.label === "string")
    .map((view) => ({
      ...view,
      kind: view.kind === "bar_with_marker" || view.kind === "bar" ? view.kind : "bar",
      value_metric: typeof view.value_metric === "string" ? view.value_metric : "value",
      error_metric: typeof view.error_metric === "string" ? view.error_metric : null,
      marker_metric: typeof view.marker_metric === "string" ? view.marker_metric : null,
      delta_metric: typeof view.delta_metric === "string" ? view.delta_metric : null,
      tooltip_metrics: asArray(view.tooltip_metrics).filter((metric) => typeof metric === "string"),
    }));

export const resolveHistogramView = (views, selectedId) => {
  const normalized = normalizeHistogramViews(views);
  if (normalized.length === 0) {
    return {
      id: "value",
      label: "Value",
      kind: "bar",
      value_metric: "value",
      error_metric: "error",
    };
  }
  return (
    normalized.find((view) => view.id === selectedId) ||
    normalized.find((view) => view.default === true) ||
    normalized[0]
  );
};

export const histogramControlEnabled = (controls, key, fallback) => {
  if (!isObject(controls)) return fallback;
  return controls[key] === true;
};

export const histogramControlDefault = (controls, key, fallback) => {
  if (!isObject(controls)) return fallback;
  return controls[key] ?? fallback;
};

export const metricDescriptor = (descriptors, metricName) => {
  if (!isObject(descriptors) || typeof metricName !== "string") return null;
  const descriptor = descriptors[metricName];
  return isObject(descriptor) ? descriptor : null;
};

export const metricLabel = (descriptors, metricName, fallback = null) => {
  const descriptor = metricDescriptor(descriptors, metricName);
  if (typeof descriptor?.short_label === "string" && descriptor.short_label.trim()) {
    return descriptor.short_label.trim();
  }
  if (typeof descriptor?.label === "string" && descriptor.label.trim()) {
    return descriptor.label.trim();
  }
  return fallback;
};

export const metricFormat = (descriptors, metricName, value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "n/a";
  const descriptor = metricDescriptor(descriptors, metricName);
  if (descriptor?.format === "percent") return `${formatScientific(numeric * 100, 5)}%`;
  return formatScientific(numeric, 6);
};

export const projectBinsForHistogramView = (bins, selectedView) =>
  asArray(bins)
    .map((bin) => {
      const nextValue = metricNumber(bin, selectedView?.value_metric);
      if (nextValue == null) return null;
      const nextError =
        selectedView?.error_metric != null
          ? (metricNumber(bin, selectedView.error_metric) ?? metricError(bin, selectedView.value_metric) ?? 0)
          : 0;
      return {
        ...bin,
        value: nextValue,
        error: Math.abs(nextError),
        source_value: bin.value,
        source_error: bin.error,
      };
    })
    .filter(Boolean);

export const discreteBinInfoLines = (bin, selectedView = null, metricDescriptors = null) => {
  const viewId = selectedView?.id || "value";
  const tooltipMetrics =
    selectedView?.tooltip_metrics?.length > 0
      ? selectedView.tooltip_metrics
      : viewId === "value"
        ? []
        : [selectedView?.value_metric, selectedView?.marker_metric, selectedView?.delta_metric].filter(Boolean);
  const lines = [];
  for (const metricName of tooltipMetrics) {
    const metricValue = metricNumber(bin, metricName);
    if (metricValue == null) continue;
    const label = metricLabel(metricDescriptors, metricName, metricName);
    const error = metricError(bin, metricName);
    lines.push(
      `${label}: ${metricFormat(metricDescriptors, metricName, metricValue)}${
        error != null ? ` +/- ${metricFormat(metricDescriptors, metricName, error)}` : ""
      }`,
    );
  }
  if (lines.length === 0 && typeof bin?.pdf_status === "string" && bin.pdf_status !== "available") {
    lines.push(`${metricLabel(metricDescriptors, "pdf", "pdf")}: ${bin.pdf_status}`);
  }
  return lines;
};
