import { formatScientific } from "../../utils/formatters";

export const gridColor = "rgba(148,163,184,0.18)";

export const baseCartesianGrid = {
  left: 56,
  right: 20,
  top: 12,
  bottom: 48,
};

export const formatAxisValue = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? formatScientific(numeric, 3) : "";
};

export const inferXAxisLabel = (panelId) =>
  String(panelId || "").includes("_history") ? "Nr samples" : null;

export const baseAxisLabel = {
  color: "#64748b",
  fontSize: 12,
  formatter: formatAxisValue,
};

export const buildErrorBarSeries = ({ name = "error", data, color = "#7c8a96", capPx = 4 }) => ({
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
    const right = left + Number(coordSys.width);
    const top = Number(coordSys.y);
    const bottom = top + Number(coordSys.height);
    if (![left, right, top, bottom].every(Number.isFinite)) return null;
    if (xPx < left || xPx > right) return null;
    if ((yLowPx < top && yHighPx < top) || (yLowPx > bottom && yHighPx > bottom)) return null;
    const y1 = Math.max(top, Math.min(bottom, yLowPx));
    const y2 = Math.max(top, Math.min(bottom, yHighPx));
    const capLeft = Math.max(left, xPx - capPx);
    const capRight = Math.min(right, xPx + capPx);
    const line = (x1, y1, x2, y2) => ({
      type: "line",
      shape: { x1, y1, x2, y2 },
      style: { stroke: color, lineWidth: 1.2 },
    });
    return {
      type: "group",
      children: [line(xPx, y1, xPx, y2), line(capLeft, y1, capRight, y1), line(capLeft, y2, capRight, y2)],
    };
  },
});
