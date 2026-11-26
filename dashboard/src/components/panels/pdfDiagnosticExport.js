import { diagnosticHistograms } from "./pdfDiagnostics";
import { formatScientific } from "../../utils/formatters";
import {
  densityColor,
  mismatchColor,
  comparisonModels,
  curveOption,
  distributionOption,
  planeOption,
  mismatchValue,
} from "./pdfDisplay";
import { PDF_PLOT_LABELS } from "./PdfExportMenu";
import { downloadBlob, figureCsv } from "./FigureExportActions";
import { prefixLabel } from "./strataUtils";

const slug = (name) => name.replace(/[^a-zA-Z0-9._-]/g, "_");
export const diagnosticExportData = (state, settings, scope, plot, zoom) => {
  const models = comparisonModels(state, scope, settings),
    c = models.c;
  const indices = Array.from(
    { length: state.data.integrand_values.length },
    (_, i) => i,
  );
  const visible = (i) => {
    if (!zoom || plot.endsWith("Histogram")) return true;
    const x = i % c.axes.x.length,
      y = Math.floor(i / c.axes.x.length);
    const fraction = (v, n) => (n > 1 ? (100 * v) / (n - 1) : 50);
    return (
      fraction(x, c.axes.x.length) >= zoom.x.start &&
      fraction(x, c.axes.x.length) <= zoom.x.end &&
      (!c.axes.y ||
        (fraction(y, c.axes.y.length) >= zoom.y.start &&
          fraction(y, c.axes.y.length) <= zoom.y.end))
    );
  };
  const histograms = plot.endsWith("Histogram")
    ? diagnosticHistograms(
        plot === "mismatchHistogram"
          ? [models.mismatch.values]
          : [c.target, c.pdf],
        40,
        plot === "densityHistogram" && settings.histogramLog,
        settings.histogramWeighted !== false ? c.joint : null,
      ).map((bins, i) => ({
        name:
          plot === "mismatchHistogram" ? "Mismatch" : ["Target", "Sampler"][i],
        bins,
      }))
    : null;
  return {
    histograms,
    kind: "pdf_diagnostic_export",
    plot,
    scope,
    prefixes: state.data.strata?.prefixes ?? null,
    pdf_steps: c.stepPoints,
    projection: state.data.projection,
    reference: state.data.reference || {
      value: state.data.global_abs_integrand_norm,
    },
    geometry: state.geometry,
    settings,
    zoom,
    normalization: settings.normalization,
    target_divisor: c.targetDivisor,
    pdf_divisor: c.pdfDivisor,
    stratum_probability: c.probability,
    // Infinite mismatches are explicit strings in JSON rather than silently serialized as null.
    points: indices.filter(visible).map((i) => ({
      x: c.axes.x[i % c.axes.x.length],
      y: c.axes.y?.[Math.floor(i / c.axes.x.length)] ?? null,
      raw_projection: state.data.integrand_values[i],
      raw_pdf: c.joint[i],
      target: c.target[i],
      pdf: c.pdf[i],
      relative_mismatch:
        mismatchValue(c, i, "relative") === Infinity
          ? "+Infinity"
          : mismatchValue(c, i, "relative"),
      log_ratio:
        mismatchValue(c, i, "log_ratio") === Infinity
          ? "+Infinity"
          : mismatchValue(c, i, "log_ratio") === -Infinity
            ? "-Infinity"
            : mismatchValue(c, i, "log_ratio"),
      projection_sign: c.sign[i],
      coverage: c.status[i],
      actual_stratum: c.actual[i] ?? null,
    })),
  };
};
const csv = (payload) => {
  if (payload.histograms)
    return figureCsv({
      columns: [
        "quantity",
        "start",
        "stop",
        "density",
        "normalization",
        "reference",
        "weighting",
      ],
      rows: payload.histograms.flatMap((h) =>
        h.bins.map((b) => [
          h.name,
          b.start,
          b.stop,
          b.value,
          payload.normalization,
          payload.reference,
          payload.settings.histogramWeighted !== false
            ? "sampler_density"
            : "uniform",
        ]),
      ),
    });
  const fields = [
    "x",
    "y",
    "raw_projection",
    "raw_pdf",
    "target",
    "pdf",
    "relative_mismatch",
    "log_ratio",
    "projection_sign",
    "coverage",
    "actual_stratum",
    "normalization",
    "reference",
    "target_divisor",
    "pdf_divisor",
    "stratum_probability",
  ];
  return figureCsv({
    columns: fields,
    rows: payload.points.map((p) =>
      fields.map((k) => p[k] ?? payload[k] ?? null),
    ),
  });
};
const optionFor = (state, settings, scope, plot, zoom) => {
  const models = comparisonModels(state, scope, settings),
    view = zoom ? { zoom: zoom.x, yZoom: zoom.y } : {};
  if (plot === "densityHistogram" || plot === "mismatchHistogram")
    return distributionOption(models, settings, plot === "mismatchHistogram");
  if (state.geometry.kind === "line")
    return curveOption(
      models,
      { ...settings, auxiliary: plot },
      plot !== "density",
      view,
    );
  const model = models[plot],
    option = planeOption(model, view);
  if (model.kind !== "sign") {
    const { min, max } = model.limits;
    const labels = [min, max].map((v) =>
      formatScientific(model.log ? 10 ** v : v, 3),
    );
    option.graphic = [
      {
        type: "group",
        left: "center",
        bottom: 14,
        children: [
          ...Array.from({ length: 64 }, (_, i) => ({
            type: "rect",
            shape: { x: i * 6, y: 0, width: 6.1, height: 14 },
            style: {
              fill:
                model.kind === "mismatch"
                  ? mismatchColor(min + ((max - min) * i) / 63, model.limits)
                  : densityColor(
                      model.log
                        ? 10 ** (min + ((max - min) * i) / 63)
                        : min + ((max - min) * i) / 63,
                      model.limits,
                      model.log,
                      model.palette,
                    ),
            },
          })),
          {
            type: "text",
            style: { text: labels[0], x: 0, y: 18, fontSize: 12 },
          },
          {
            type: "text",
            style: {
              text: labels[1],
              x: 384,
              y: 18,
              align: "right",
              fontSize: 12,
            },
          },
          ...(model.kind === "mismatch" && min < 0 && max > 0
            ? [
                {
                  type: "text",
                  style: {
                    text: "0",
                    x: (384 * -min) / (max - min),
                    y: 18,
                    fontSize: 12,
                  },
                },
              ]
            : []),
        ],
      },
    ];
  }
  return option;
};
const svgFor = async (option, title, description) => {
  const { echarts } = await import("../../lib/echarts");
  const chart = echarts.init(null, null, {
    renderer: "svg",
    ssr: true,
    width: 1000,
    height: 650,
  });
  try {
    chart.setOption({
      ...option,
      title: { text: title, subtext: description, left: "center" },
      grid: { ...option.grid, top: 75, bottom: 100 },
      backgroundColor: "#fff",
      animation: false,
      dataZoom: (option.dataZoom || []).map((z) => ({ ...z, show: false })),
    });
    return chart.renderToSVGString();
  } finally {
    chart.dispose();
  }
};
const pngFor = async (svg) => {
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = 2000;
    canvas.height = 1300;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
    return await new Promise((resolve, reject) =>
      canvas.toBlob(
        (blob) =>
          blob ? resolve(blob) : reject(new Error("PNG export failed.")),
        "image/png",
      ),
    );
  } finally {
    URL.revokeObjectURL(url);
  }
};
export const exportPdfDiagnostics = async ({
  state,
  settings,
  scopes,
  plots,
  format,
  zoom,
}) => {
  const files = {},
    pages = [];
  for (const scope of scopes)
    for (const plot of plots) {
      const name = `${plot}-${scope === "overall" ? "overall" : `stratum-${prefixLabel(state.data.strata.prefixes[Number(scope)])}`}`;
      const title = `${PDF_PLOT_LABELS[plot]} — ${scope === "overall" ? "Overall" : prefixLabel(state.data.strata.prefixes[Number(scope)])}`;
      if (format === "json" || format === "csv") {
        const payload = diagnosticExportData(
          state,
          settings,
          scope,
          plot,
          zoom,
        );
        files[`${slug(name)}.${format}`] = new TextEncoder().encode(
          format === "json" ? JSON.stringify(payload, null, 2) : csv(payload),
        );
      } else {
        const ref = state.data.reference;
        const description = `${settings.normalization === "slice" ? "Slice: mean-one fields" : `Source integral: ${ref?.value ?? state.data.global_abs_integrand_norm}; SE: ${ref?.error ?? "unavailable"}`} · ${state.data.target_label || "Absolute training projection"}`;
        const svg = await svgFor(
          optionFor(state, settings, scope, plot, zoom),
          title,
          description,
        );
        if (format === "pdf") pages.push({ svg, title });
        else
          files[`${slug(name)}.${format}`] =
            format === "svg"
              ? new TextEncoder().encode(svg)
              : new Uint8Array(await (await pngFor(svg)).arrayBuffer());
      }
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  if (format === "pdf") {
    const { jsPDF } = await import("jspdf"),
      { svg2pdf } = await import("svg2pdf.js"),
      { registerPdfFont } = await import("./pdfFont");
    const pdf = new jsPDF({
      orientation: "landscape",
      unit: "pt",
      format: "a4",
      compress: true,
    });
    await registerPdfFont(pdf);
    for (const [i, { svg }] of pages.entries()) {
      if (i) pdf.addPage();
      const element = new DOMParser().parseFromString(
        svg,
        "image/svg+xml",
      ).documentElement;
      element.querySelectorAll("text").forEach((text) => {
        text.style.fontFamily = "GammaBoard";
      });
      await svg2pdf(element, pdf, {
        x: 20,
        y: 20,
        width: 800,
        height: 520,
        loadExternalStyleSheets: false,
      });
    }
    downloadBlob("pdf-diagnostics.pdf", pdf.output("blob"));
    return;
  }
  const entries = Object.entries(files);
  if (entries.length === 1) {
    const [name, data] = entries[0];
    downloadBlob(
      name,
      new Blob([data], {
        type: {
          svg: "image/svg+xml",
          png: "image/png",
          json: "application/json",
          csv: "text/csv",
        }[format],
      }),
    );
  } else {
    const { zipSync } = await import("fflate");
    downloadBlob(
      "pdf-diagnostics.zip",
      new Blob([zipSync(files)], { type: "application/zip" }),
    );
  }
};
