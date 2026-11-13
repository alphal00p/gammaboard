import { finiteNumberOrNull } from "./histogramViews";

export const OBSERVABLE_DISPLAYS = [
  { id: "estimate", label: "Estimate" },
  { id: "compare_estimates", label: "Compare estimates" },
  { id: "errors", label: "Compare errors" },
  { id: "error_reduction", label: "Error reduction" },
];
export const stratumKey = (prefix) => prefix.join(",");

export const resolveObservableView = (state, requested = {}) => {
  const stratum = state.strata?.some((s) => stratumKey(s.prefix) === requested.stratum)
    ? requested.stratum : "combined";
  return {
    stratum,
    estimator: stratum === "combined" && requested.estimator === "pooled" ? "pooled" : "stratified",
    display: OBSERVABLE_DISPLAYS.some((v) => v.id === requested.display) ? requested.display : "estimate",
  };
};

// No estimates are recomputed here. The host provides conditional and pooled
// contributions on the same bin layout, with unavailable values left null.
export const observableHistogramView = (state, requested) => {
  const selection = resolveObservableView(state, requested);
  const stratum = state.strata?.find((s) => stratumKey(s.prefix) === selection.stratum);
  const source = state.bins.map((bin, index) => {
    const pair = stratum?.bins[index];
    const estimate = stratum ? pair?.estimate : { mean: bin.value, error: bin.error };
    const pooled = stratum ? pair?.pooled : bin.pooled;
    const primaryError = finiteNumberOrNull(estimate?.error);
    const pooledError = finiteNumberOrNull(pooled?.error);
    return {
      ...bin,
      // Pooled raw diagnostics belong to the combined view, not an individual stratum.
      entry_count: stratum ? null : bin.entry_count,
      value: finiteNumberOrNull(estimate?.mean),
      error: primaryError,
      pooled,
      comparison: {
        stratified_mean: finiteNumberOrNull(estimate?.mean),
        stratified_error: primaryError,
        pooled_mean: finiteNumberOrNull(pooled?.mean),
        pooled_error: pooledError,
        error_reduction: primaryError > 0 && pooledError != null ? pooledError / primaryError : null,
      },
    };
  });
  const display = selection.display;
  const errorView = display === "errors" || display === "error_reduction";
  const curveLabel = (estimator) => {
    if (estimator === "pooled") return errorView ? "Pooled error" : "Pooled";
    if (stratum) return errorView ? "Stratum error" : "Stratum contribution";
    return errorView ? "Stratified error" : "Stratified";
  };
  const curveBins = (estimator) => source.map((bin) => ({
    ...bin,
    value: bin.comparison[`${estimator}_${errorView ? "error" : "mean"}`],
    error: errorView ? null : bin.comparison[`${estimator}_error`],
  }));
  const primaryLabel = display === "error_reduction" ? "Pooled / stratified error"
    : curveLabel(selection.estimator);
  const bins = display === "error_reduction"
    ? source.map((bin) => ({ ...bin, value: bin.comparison.error_reduction, error: null }))
    : curveBins(selection.estimator);
  const otherEstimator = selection.estimator === "pooled" ? "stratified" : "pooled";
  const overlay = display === "compare_estimates" || display === "errors" ? [{
    name: curveLabel(otherEstimator),
    color: "#d97706",
    bins: curveBins(otherEstimator),
  }] : [];
  return {
    selection, stratum, errorView,
    state: {
      ...state,
      bins,
      estimator: selection.estimator,
      series_label: primaryLabel,
      y_label: display === "errors" ? "Standard error" : display === "error_reduction" ? "Error reduction" : "Contribution",
      sample_count: stratum?.sample_count ?? state.sample_count,
      selection,
      reference_value: display === "error_reduction" ? 1 : undefined,
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
