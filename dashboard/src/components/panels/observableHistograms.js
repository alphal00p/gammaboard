import { finiteNumberOrNull } from "./histogramViews";

export const OBSERVABLE_DISPLAYS = [
  { id: "estimate", label: "Estimate" },
  { id: "compare_estimates", label: "Compare estimates" },
  { id: "errors", label: "Compare errors" },
  { id: "error_ratio", label: "Error reduction" },
];
export const stratumKey = (prefix) => prefix.join(",");

export const resolveObservableView = (state, requested = {}) => ({
  stratum: state.strata?.some((s) => stratumKey(s.prefix) === requested.stratum)
    ? requested.stratum : "combined",
  display: OBSERVABLE_DISPLAYS.some((v) => v.id === requested.display) ? requested.display : "estimate",
});

// No estimates are recomputed here. The host provides conditional and pooled
// contributions on the same bin layout, with unavailable values left null.
export const observableHistogramView = (state, requested) => {
  const selection = resolveObservableView(state, requested);
  const stratum = state.strata?.find((s) => stratumKey(s.prefix) === selection.stratum);
  const source = state.bins.map((bin, index) => {
    const pair = stratum?.bins[index];
    const estimate = stratum ? pair?.estimate : { mean: bin.value, error: bin.error };
    const basic = stratum ? pair?.basic : bin.basic;
    const primaryError = finiteNumberOrNull(estimate?.error);
    const basicError = finiteNumberOrNull(basic?.error);
    return {
      ...bin,
      // Pooled raw diagnostics belong to the combined view, not an individual stratum.
      entry_count: stratum ? null : bin.entry_count,
      value: finiteNumberOrNull(estimate?.mean),
      error: primaryError,
      basic,
      comparison: {
        stratified_mean: finiteNumberOrNull(estimate?.mean),
        stratified_error: primaryError,
        basic_mean: finiteNumberOrNull(basic?.mean),
        basic_error: basicError,
        error_ratio: primaryError > 0 && basicError != null ? basicError / primaryError : null,
      },
    };
  });
  const display = selection.display;
  const errorView = display === "errors" || display === "error_ratio";
  const primaryLabel = display === "errors" ? "Stratified error"
    : display === "error_ratio" ? "Basic / stratified error" : "Stratified";
  const bins = source.map((bin) => ({
    ...bin,
    value: display === "errors" ? bin.error : display === "error_ratio" ? bin.comparison.error_ratio : bin.value,
    error: errorView ? null : bin.error,
  }));
  const overlay = display === "compare_estimates" || display === "errors" ? [{
    name: display === "errors" ? "Basic error" : "Basic (unstratified)",
    color: "#d97706",
    bins: source.map((bin) => ({ ...bin, value: display === "errors" ? bin.comparison.basic_error : bin.comparison.basic_mean, error: display === "errors" ? null : bin.comparison.basic_error })),
  }] : [];
  return {
    selection, stratum, errorView,
    state: {
      ...state,
      bins,
      series_label: primaryLabel,
      y_label: display === "errors" ? "Standard error" : display === "error_ratio" ? "Error reduction" : "Contribution",
      sample_count: stratum?.sample_count ?? state.sample_count,
      selection,
      reference_value: display === "error_ratio" ? 1 : undefined,
      zero_baseline: errorView,
      stratum_prefix: stratum?.prefix ?? null,
      overlay_alignment: "shared_edges",
      overlay_histograms: overlay,
      controls: {
        scale: true, x_scale: true, sort: true, export: true, reset_view: true,
        pdf_cdf: !errorView, relative_error: !errorView, default_relative_error: false,
        // These estimators use the same draws: generic independent ratio errors do not apply.
        ratio: display === "estimate" && !stratum,
      },
    },
  };
};
