import { useState } from "react";
import { Alert, Button, Stack } from "@mui/material";

export const downloadTextFile = (filename, contents, mimeType = "text/plain;charset=utf-8") => {
  const blob = new Blob([contents], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = "noreferrer";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
};

const sanitizeFigureFilename = (value, fallback = "figure") => {
  const text = String(value ?? "").trim();
  const normalized = text
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return normalized || fallback;
};

export const escapeXml = (value) =>
  String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");

const downloadJsonFile = (baseName, payload) => {
  downloadTextFile(
    `${sanitizeFigureFilename(baseName)}.json`,
    `${JSON.stringify(payload, null, 2)}\n`,
    "application/json;charset=utf-8",
  );
};

const downloadSvgFromElement = (baseName, element) => {
  const svg = element?.querySelector?.("svg");
  if (!svg) return false;
  const serializer = new XMLSerializer();
  const cloned = svg.cloneNode(true);
  cloned.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  const markup = serializer.serializeToString(cloned);
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
  return true;
};

// Render a separate SVG instance so canvas plots export actual vector paths.
const exportCharts = async (baseName, echartsRef, element) => {
  const { echarts } = await import("../../lib/echarts");
  const direct = echartsRef?.current?.getEchartsInstance?.();
  const instances = direct ? [direct] : Array.from(element?.querySelectorAll?.("[_echarts_instance_]") || [])
    .map((node) => echarts.getInstanceByDom(node)).filter(Boolean);
  if (!instances.length) return false;
  let height = 0;
  let width = 1;
  const figures = instances.map((instance) => {
    const w = instance.getWidth();
    const h = instance.getHeight();
    const renderer = echarts.init(null, null, { renderer: "svg", ssr: true, width: w, height: h });
    try {
      renderer.setOption({ ...instance.getOption(), animation: false });
      const svg = renderer.renderToSVGString().replace("<svg ", `<svg y="${height}" `);
      height += h;
      width = Math.max(width, w);
      return svg;
    } finally { renderer.dispose(); }
  });
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`,
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">${figures.join("")}</svg>`, "image/svg+xml;charset=utf-8");
  return true;
};

const downloadPng = (baseName, element) => {
  const canvas = element?.querySelector?.("canvas");
  if (!canvas) return false;
  const anchor = document.createElement("a");
  anchor.href = canvas.toDataURL("image/png");
  anchor.download = `${sanitizeFigureFilename(baseName)}.png`;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  return true;
};

const FigureExportActions = ({
  baseName,
  payload,
  elementRef = null,
  echartsRef = null,
  svgBuilder = null,
  imageFormat = "svg",
  onResetView = null,
}) => {
  const [error, setError] = useState(null);
  const handleDownloadImage = async () => {
    setError(null);
    try {
      if (imageFormat === "png") {
        if (downloadPng(baseName, elementRef?.current)) return;
      } else {
        if (typeof svgBuilder === "function") {
          const markup = svgBuilder();
          if (typeof markup === "string" && markup.trim()) {
            downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
            return;
          }
        }
        if (await exportCharts(baseName, echartsRef, elementRef?.current)) return;
        if (downloadSvgFromElement(baseName, elementRef?.current)) return;
      }
      setError("The figure is not ready to export yet.");
    } catch (err) { setError(`Export failed: ${err.message}`); }
  };

  return (
    <Stack direction="row" spacing={1} alignItems="center">
      {typeof onResetView === "function" ? (
        <Button size="small" variant="outlined" onClick={onResetView}>
          Reset
        </Button>
      ) : null}
      <Button size="small" variant="outlined" onClick={() => downloadJsonFile(baseName, payload)}>
        JSON
      </Button>
      <Button size="small" variant="outlined" onClick={handleDownloadImage}>
        {imageFormat.toUpperCase()}
      </Button>
      {error && <Alert severity="error">{error}</Alert>}
    </Stack>
  );
};

export default FigureExportActions;
