import { Component, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { jsPDF } from "jspdf";
import { svg2pdf } from "svg2pdf.js";
import { echarts } from "../../lib/echarts";
import ObservableHistogramPanel from "./ObservableHistogramPanel";
import { registerPdfFont } from "./pdfFont";
import { histogramSelectionKey, histogramSupportsComparison, histogramIsDiscrete,
  buildHistogramData, normalizeGammaLoopHistogramBins } from "./histogramUtils";

// Preserve configured selections (including an intentionally empty list). For
// unvisited observables, use the same visible-title match as Add Histogram.
export const pdfComparisonBundles = (bundles, name, histogram) => bundles.map((bundle) => {
  const key = histogramSelectionKey(name);
  if (Object.hasOwn(bundle.selectionsByHistogram || {}, key)) return bundle;
  const names = Object.keys(bundle.histograms).filter((entry) =>
    histogramSupportsComparison(bundle.histograms[entry], histogramIsDiscrete(histogram)));
  const selected = names.includes(histogram.title) ? histogram.title : names[0];
  return { ...bundle, selectionsByHistogram: { ...bundle.selectionsByHistogram, [key]: {
    selectedHistograms: selected == null ? [] : [selected],
    discreteAlignmentByHistogram: selected == null ? {} : {
      [selected]: bundle.histograms[selected].source_format === "hwu" ? "by_index" : "by_key",
    },
  } } };
});

function CapturePlots({ plots, onReady }) {
  useEffect(() => { onReady(plots); }, [plots, onReady]);
  return null;
}

class PlotErrorBoundary extends Component {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error) { this.props.onError(error); }
  render() { return this.state.failed ? null : this.props.children; }
}

const checkAborted = (signal) => { if (signal?.aborted) throw new DOMException("Export cancelled", "AbortError"); };

export const renderHistogramBundlePdf = async ({ payload, panelId, value, uploadedBundles = [],
  includeComparisons = false, onProgress = () => {}, signal }) => {
  const entries = Object.entries(payload.histograms || {});
  if (!entries.length) throw new Error("No observables available to export.");
  const pdf = new jsPDF({ unit: "pt", format: "a4", compress: true, putOnlyUsedFonts: true });
  pdf.setProperties({ title: "Observable plots", creator: "GammaBoard" });
  await registerPdfFont(pdf, signal);
  const root = createRoot(document.createElement("div"));
  const width = pdf.internal.pageSize.getWidth();
  const height = pdf.internal.pageSize.getHeight();
  const margin = 32;
  try {
    for (const [index, [name, histogram]] of entries.entries()) {
      checkAborted(signal);
      onProgress(index + 1, entries.length);
      const bins = histogram.bins?.some((bin) => bin?.value != null)
        ? buildHistogramData(histogram.bins) : normalizeGammaLoopHistogramBins(histogram);
      const bundles = includeComparisons ? pdfComparisonBundles(uploadedBundles, name, histogram) : [];
      const plots = await new Promise((resolve, reject) => {
        const abort = () => reject(new DOMException("Export cancelled", "AbortError"));
        const finish = (callback) => (result) => { signal?.removeEventListener("abort", abort); callback(result); };
        signal?.addEventListener("abort", abort, { once: true });
        root.render(<PlotErrorBoundary key={name} onError={finish(reject)}>
          <ObservableHistogramPanel state={{ ...histogram, bins, name, panel_id: `${panelId}_selected`, source_panel_id: panelId,
              strata: payload.strata?.map((stratum, k) => ({ ...stratum, ...histogram.strata?.[k] })).filter((stratum) => Array.isArray(stratum.bins)),
            }}
            value={value} onValueChange={() => {}} uploadedBundles={bundles}
            renderPlots={(model) => <CapturePlots plots={model} onReady={finish(resolve)} />} />
        </PlotErrorBoundary>);
      });
      checkAborted(signal);
      if (plots.errors.length) throw new Error(`${histogram.title || name}: ${plots.errors.join(" ")}`);
      if (index > 0) pdf.addPage();
      pdf.setFontSize(15);
      const title = pdf.splitTextToSize(histogram.title || name, width - margin * 2);
      pdf.text(title, margin, 40);
      let top = 52 + (title.length - 1) * 18;
      pdf.setFontSize(9);
      const notes = [payload.estimator_note,
        plots.approximateErrors ? "Merged-bin errors assume uncorrelated bins." : null,
        plots.cumulativeErrorsUnavailable ? "Cumulative uncertainties are unavailable without covariance between bins." : null,
        plots.unmatchedComparison ? "Comparison ratios require matching intervals or exact sums of finer bins." : null,
        payload.omitted_incompatible_histograms?.length ? `${payload.omitted_incompatible_histograms.length} incompatible observables omitted from the campaign result.` : null,
      ].filter(Boolean).flatMap((note) => pdf.splitTextToSize(note, width - margin * 2));
      const bottom = height - 40 - notes.length * 11;
      const charts = [
        { option: plots.histogramOption, weight: 2 },
        ...(plots.relativeOption ? [{ option: plots.relativeOption, label: "Relative Error", weight: 1 }] : []),
        ...(plots.ratioOption ? [{ option: plots.ratioOption, label: "1 - value / comparison", weight: 1 }] : []),
      ];
      const totalWeight = charts.reduce((sum, chart) => sum + chart.weight, 0);
      const availableHeight = bottom - top - charts.length * 20;
      if (availableHeight < 120) throw new Error(`Title or annotations are too long to fit '${name}' on one PDF page.`);
      if (!plots.bins.length) {
        pdf.text("No observable bins available.", margin, top + 30);
      } else for (const { option, label, weight } of charts) {
        checkAborted(signal);
        if (label) { pdf.text(label, margin, top + 10); top += 16; }
        const chartHeight = availableHeight * weight / totalWeight;
        const renderer = echarts.init(null, null, { renderer: "svg", ssr: true,
          width: (width - margin * 2) * 1.5, height: chartHeight * 1.5 });
        try {
          renderer.setOption({ ...option, animation: false, backgroundColor: "#fff",
            yAxis: label ? { ...option.yAxis, name: "" } : option.yAxis,
            dataZoom: (option.dataZoom || []).map((zoom) => ({ ...zoom, show: false })),
          });
          const svg = new DOMParser().parseFromString(renderer.renderToSVGString(), "image/svg+xml").documentElement;
          svg.querySelectorAll("text").forEach((text) => { text.style.fontFamily = "GammaBoard"; });
          await svg2pdf(svg, pdf, { x: margin, y: top, width: width - margin * 2, height: chartHeight,
            loadExternalStyleSheets: false, loadImages: false });
        } finally { renderer.dispose(); }
        top += chartHeight + 4;
      }
      if (notes.length) pdf.text(notes, margin, bottom + 14);
      pdf.text(`${index + 1} / ${entries.length}`, width - margin, height - 18, { align: "right" });
      // Yield between pages so progress and cancellation remain responsive.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    checkAborted(signal);
    return pdf.output("blob");
  } finally { root.unmount(); }
};
