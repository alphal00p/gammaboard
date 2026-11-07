export const TASK_OUTPUT_TABS = [["result", "Result"], ["observables", "Observables"], ["strata", "Stratum output"], ["diagnostics", "Diagnostics"]];
const observables = new Set(["gammaloop_histogram_bundle", "campaign_histograms", "discrete_projection_bundle"]);
const diagnostics = new Set(["gammaloop_evaluation_diagnostics", "gammaloop_evaluation_timing", "max_weight_points", "max_weight_summary"]);
const configuration = new Set(["evaluator_summary", "evaluator_config", "sampler_summary", "sampler_config", "engine_config_error"]);

// Unknown task output stays visible in Result. Group by stable IDs, not labels
// or panel types: campaign tables and scan heatmaps are results too.
export function taskPanelSection(panel) {
  if (panel.panel_id === "task_summary") return "summary";
  if (observables.has(panel.panel_id)) return "observables";
  if (["strata", "stratum_results"].includes(panel.panel_id)) return "strata";
  if (diagnostics.has(panel.panel_id)) return "diagnostics";
  return "result";
}
export const runPanelSection = (panel) => configuration.has(panel.panel_id) ? "settings"
  : ["run_queue", "run_batch"].includes(panel.panel_id) ? "tuning" : "summary";
