import { rasterAxes } from './pdfDiagnostics';
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

const csvCell = (value) => {
  if (value == null) return "";
  const text = typeof value === "object" ? JSON.stringify(value) : String(value);
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
};

export const figureCsv = (payload) => {
  const state = payload?.state || payload;
  let columns;
  let rows;
  if (state?.kind === "strata") {
    columns = ["scope","samples","wall_time_ms","sampler_uptime_ms","estimator","prefix","component","attempts","matches","exact_probability","mean","error","basic_mean","basic_error"];
    rows = state.frames.flatMap(frame => [
      ...frame.rows.flatMap(row => state.components.map((component,j) => ["stratum",frame.samples,frame.wall_time_ms,frame.sampler_uptime_ms,state.estimator,row.prefix,component,row.attempts,row.matches,row.probability,row.estimates[j]?.mean,row.estimates[j]?.error,null,null])),
      ...state.components.map((component,j) => ["total",frame.samples,frame.wall_time_ms,frame.sampler_uptime_ms,state.estimator,null,component,frame.samples,null,null,frame.total[j]?.mean,frame.total[j]?.error,frame.basic?.[j]?.mean,frame.basic?.[j]?.error]),
    ]);
  } else if (state?.kind === "pdf_comparison") {
    const {data,geometry}=state, axes=rasterAxes(geometry);
    const prefixes=data.strata?.prefixes||[[]];
    columns=["record_type","x","x_end","y","prefix","exact_probability","actual_prefix","unmasked_projection","joint_pdf","conditional_pdf","target_label","source_integral_reference"];
    rows=prefixes.flatMap((prefix,k)=>{
      const p=data.strata?data.strata.probabilities[k]:1;
      const grid=data.integrand_values.map((v,i)=>{
        const pdf=data.strata?data.strata.pdf_values[k][i]:data.pdf_values[i];
        const actual=data.strata?.actual_strata[i];
        return ["grid",axes.x[i%axes.x.length],null,axes.y?.[Math.floor(i/axes.x.length)],prefix,p,actual==null?null:prefixes[actual],v,pdf,p!=null&&p>0&&pdf!=null?pdf/p:null,data.target_label,data.global_abs_integrand_norm];
      });
      const steps=data.pdf_steps?.densities[k]?.map((pdf,i)=>["pdf_interval",data.pdf_steps.edges[i],data.pdf_steps.edges[i+1],null,prefix,p,null,null,pdf,p!=null&&p>0&&pdf!=null?pdf/p:null,data.target_label,data.global_abs_integrand_norm])||[];
      return [...grid,...steps];
    });
  } else if (Array.isArray(state?.points) && state.points.some(point => Object.hasOwn(point,"pdf"))) {
    columns = ["record_type","x","y","target","pdf","actual_stratum","normalization","relative_mismatch","log_ratio","coverage"];
    rows = state.points.map(point=>["grid",point.x,point.y,point.target,point.pdf,point.actual_stratum,state.normalization,point.relative_mismatch,point.log_ratio,point.coverage]);
    for (const [x,pdf] of state.pdf_steps||[]) rows.push(["pdf_step",x,null,null,pdf,null,state.normalization,null,null,null]);
  } else if (Array.isArray(state?.rows) && Array.isArray(state?.columns)) {
    columns = [...state.columns];
    rows = state.rows.map((row) => [...row]);
    for (const key of ["estimator", "weight_basis"]) {
      if (state.payload?.[key] && !columns.includes(key)) {
        columns.push(key);
        rows.forEach((row) => row.push(state.payload[key]));
      }
    }
  } else {
    columns = ["quantity", "x", "estimator", "value", "primary_error", "strat_err", "basic_mean", "basic_err", "uncertainty_status", "samples"];
    const row = (name, x, value, error, estimator, basic, samples) => [name, x, estimator, value, error,
      estimator === "stratified" ? error : null,
      basic?.mean ?? (estimator === "pooled" ? value : null),
      basic?.error ?? (estimator === "pooled" ? error : null),
      error == null ? "unavailable" : "estimated_from_samples", samples];
    if (Array.isArray(state?.entries)) {
      rows = state.entries.filter((entry) => entry.value?.kind === "estimate").map((entry) =>
        row(entry.label, null, entry.value.value, entry.value.error, entry.value.estimator, entry.value.basic, entry.value.sample_count));
    } else if (Array.isArray(state?.points) || Array.isArray(state?.series)) {
      const series = state.series || [{ label: payload.panel_id || "value", points: state.points }];
      rows = series.flatMap((s) => (s.points || []).map((point) => row(s.label, point.x, point.y,
        point.error,
        point.estimator, point.basic, point.x_completed_samples_total ?? (state.x_axis === "completed_samples" ? point.x : null))));
    } else return null;
  }
  if (!rows.length) return null;
  return [columns, ...rows].map((row) => row.map(csvCell).join(",")).join("\r\n") + "\r\n";
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
  const csv = figureCsv(payload);
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
      {csv != null && <Button size="small" variant="outlined" onClick={() => downloadTextFile(`${sanitizeFigureFilename(baseName)}.csv`, csv, "text/csv;charset=utf-8")}>CSV</Button>}
      {imageFormat && <Button size="small" variant="outlined" onClick={handleDownloadImage}>
        {imageFormat.toUpperCase()}
      </Button>}
      {error && <Alert severity="error">{error}</Alert>}
    </Stack>
  );
};

export default FigureExportActions;
