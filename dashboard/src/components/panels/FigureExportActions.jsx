import { Button, Stack } from "@mui/material";

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

const downloadSvgFromEcharts = (baseName, echartsRef) => {
  const instance = echartsRef?.current?.getEchartsInstance?.();
  if (!instance) return false;
  try {
    const dataUrl = instance.getDataURL({
      type: "svg",
      pixelRatio: 2,
      backgroundColor: "#ffffff",
    });
    if (typeof dataUrl !== "string" || !dataUrl.startsWith("data:image/svg+xml")) return false;
    const encoded = dataUrl.slice(dataUrl.indexOf(",") + 1);
    const markup = decodeURIComponent(encoded);
    downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
    return true;
  } catch {
    return false;
  }
};

const downloadCanvasAsSvg = (baseName, canvas) => {
  if (!canvas?.toDataURL) return false;
  const width = Number(canvas.width) || 1;
  const height = Number(canvas.height) || 1;
  const pngDataUri = canvas.toDataURL("image/png");
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`,
    `<image href="${pngDataUri}" x="0" y="0" width="${width}" height="${height}" />`,
    "</svg>",
  ].join("");
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, svg, "image/svg+xml;charset=utf-8");
  return true;
};

const downloadCanvasCollectionAsSvg = (baseName, canvases) => {
  const list = Array.from(canvases || []).filter((canvas) => canvas?.toDataURL);
  if (list.length === 0) return false;
  const width = Math.max(...list.map((canvas) => Number(canvas.width) || 1), 1);
  const heights = list.map((canvas) => Number(canvas.height) || 1);
  const totalHeight = heights.reduce((sum, height) => sum + height, 0);
  let yOffset = 0;
  const images = list
    .map((canvas, index) => {
      const height = heights[index];
      const pngDataUri = canvas.toDataURL("image/png");
      const imageTag = `<image href="${pngDataUri}" x="0" y="${yOffset}" width="${width}" height="${height}" />`;
      yOffset += height;
      return imageTag;
    })
    .join("");
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${totalHeight}" viewBox="0 0 ${width} ${totalHeight}">`,
    images,
    "</svg>",
  ].join("");
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, svg, "image/svg+xml;charset=utf-8");
  return true;
};

const FigureExportActions = ({
  baseName,
  payload,
  elementRef = null,
  echartsRef = null,
  svgBuilder = null,
  onResetView = null,
}) => {
  const handleDownloadSvg = () => {
    if (downloadSvgFromEcharts(baseName, echartsRef)) return;
    if (downloadSvgFromElement(baseName, elementRef?.current)) return;
    if (downloadCanvasCollectionAsSvg(baseName, elementRef?.current?.querySelectorAll?.("canvas"))) return;
    if (downloadCanvasAsSvg(baseName, elementRef?.current?.querySelector?.("canvas"))) return;
    if (typeof svgBuilder === "function") {
      const markup = svgBuilder();
      if (typeof markup === "string" && markup.trim()) {
        downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
      }
    }
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
      <Button size="small" variant="outlined" onClick={handleDownloadSvg}>
        SVG
      </Button>
    </Stack>
  );
};

export default FigureExportActions;
