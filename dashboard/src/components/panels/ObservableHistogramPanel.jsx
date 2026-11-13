import { useMemo, useState } from "react";
import { Autocomplete, MenuItem, TextField, Typography } from "@mui/material";
import HistogramPanel from "./HistogramPanel";
import { readHistogramBundleView, writeHistogramBundlePanelValue } from "./histogramUtils";
import { OBSERVABLE_DISPLAYS, observableHistogramView, stratumKey } from "./observableHistograms";
import { prefixLabel } from "./strataUtils";

// Keep population/estimator selection separate from the shared histogram renderer.
export default function ObservableHistogramPanel(props) {
  if (!props.state?.strata?.length) return <HistogramPanel {...props} />;
  return <StratifiedHistogram {...props} />;
}

function StratifiedHistogram({ state, value, onValueChange, ...props }) {
  const [localValue, setLocalValue] = useState({});
  const activeValue = onValueChange ? value : localValue;
  const view = readHistogramBundleView(activeValue);
  const projected = useMemo(() => observableHistogramView(state, view.observable), [state, view.observable]);
  const { selection, stratum, errorView } = projected;
  const options = useMemo(() => [
    { key: "stratified", label: "Combined · stratified", stratum: "combined", estimator: "stratified" },
    { key: "pooled", label: "Combined · pooled", stratum: "combined", estimator: "pooled" },
    ...state.strata.map((s) => ({ key: stratumKey(s.prefix), label: `Stratum ${prefixLabel(s.prefix)}`, stratum: stratumKey(s.prefix), estimator: "stratified" })),
  ], [state.strata]);
  const update = (patch) => {
    const next = writeHistogramBundlePanelValue(activeValue, { observableView: { ...selection, ...patch } });
    if (onValueChange) onValueChange(state.source_panel_id || state.panel_id, next, false);
    else setLocalValue(next);
  };
  // Suppress controls and stored display modes that have no meaning for errors.
  // Preserve the preferences so returning to estimates restores the user's view.
  const renderedValue = writeHistogramBundlePanelValue(activeValue, {
    ...(errorView ? { histogramMode: "pdf", showRelativeError: false } : { showRelativeError: view.show_relative_error === true }),
    ...(selection.display !== "estimate" || stratum ? { showRatio: false } : {}),
  });
  const updateHistogram = (_, next) => {
    if (errorView) {
      next = { ...next, histogram_view: { ...next.histogram_view, display_mode: view.display_mode, show_relative_error: view.show_relative_error } };
    }
    if (selection.display !== "estimate" || stratum) {
      next = { ...next, histogram_view: { ...next.histogram_view, show_ratio: view.show_ratio } };
    }
    if (onValueChange) onValueChange(state.source_panel_id || state.panel_id, next, false);
    else setLocalValue(next);
  };
  const subtitle = stratum
    ? `Stratum ${prefixLabel(stratum.prefix)} · ${stratum.sample_count.toLocaleString()} draws · p = ${stratum.probability.toPrecision(4)}`
    : `Combined · ${state.sample_count.toLocaleString()} draws across ${state.strata.length} strata`;
  const controls = <>
    <Autocomplete
      size="small" disableClearable
      options={options}
      value={options.find((s) => s.stratum === selection.stratum && s.estimator === selection.estimator)}
      isOptionEqualToValue={(a, b) => a.key === b.key}
      filterOptions={(items, { inputValue }) => items.filter((item) => item.label.replace(/\s/g, "").toLowerCase().includes(inputValue.replace(/\s/g, "").toLowerCase()))}
      onChange={(_, option) => update({ stratum: option.stratum, estimator: option.estimator })}
      renderInput={(params) => <TextField {...params} label="View" />}
      sx={{ width: 230 }}
    />
    <TextField select label="Display" size="small" value={selection.display}
      onChange={(event) => update({ display: event.target.value })} sx={{ minWidth: 185 }}>
      {OBSERVABLE_DISPLAYS.map((display) => <MenuItem key={display.id} value={display.id}>{display.label}</MenuItem>)}
    </TextField>
  </>;
  const caption = <>
    <Typography variant="caption" color="text.secondary" display="block">{subtitle}. Each stratum shows its contribution to the integral.</Typography>
    {(selection.estimator !== "pooled" || selection.display !== "estimate") && projected.state.bins.some((bin) => bin.comparison.stratified_error == null) && <Typography variant="caption" color="text.secondary" display="block">
      Stratified errors are pending sufficient coverage of every selected stratum.
    </Typography>}
    {(errorView || stratum) && props.uploadedBundles?.length > 0 && <Typography variant="caption" color="text.secondary" display="block">
      Uploaded reference curves are shown in the combined estimate views.
    </Typography>}
    {selection.display !== "estimate" && <Typography variant="caption" color="text.secondary" display="block">
      Pooled uses the same weighted draws, including zero contributions from stratum misses. {selection.display === "error_reduction" ? "Above 1 means stratification reduces the error; zero or unavailable stratified errors leave gaps." : "Each error belongs to its own estimator."}
    </Typography>}
  </>;
  return <HistogramPanel {...props} state={projected.state} value={renderedValue}
    onValueChange={updateHistogram} controlledView
    extraControls={controls} caption={caption}
    // Uploaded reference curves describe values, not standard errors or their ratio.
    uploadedBundles={errorView || stratum ? [] : props.uploadedBundles}
  />;
}
