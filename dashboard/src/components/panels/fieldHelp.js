const campaignHelp = {
  min_evaluators_per_run: "Minimum evaluator allocation for each scheduled child. Effective concurrency is limited by the assigned evaluator count divided by this minimum, and by sampler availability.",
  allocation_window_samples: "Maximum additional sample budget granted to a child in one scheduling window. This is a sample count, not a batch size or a time interval. A selected child normally continues until its grant is consumed; worker availability can still change placement.",
  min_samples_per_child: "Pilot samples required from each child's final publishing stage before variance-based allocation. Earlier non-publishing training samples do not satisfy this pilot minimum, but do count toward the campaign's total sample cap.",
  min_total_samples: "Minimum total samples across the campaign's children before an error target can stop the campaign. This includes earlier stages such as training.",
  max_total_samples: "Global sample budget across all child stages, including training. Work already granted reserves part of this budget even before its samples are completed.",
  max_active_runs: "Upper limit on concurrently scheduled children. Each child also needs a sampler and the configured minimum evaluator allocation, so available workers can reduce actual concurrency.",
  requested_concurrency: "Upper limit on concurrently scheduled children. Available sampler and evaluator workers can reduce actual concurrency.",
  worker_capacity: "Number of children the assigned workers can support, capped by the configured concurrency. Each child requires one sampler and at least the configured minimum number of evaluators.",
  selected_children: "Children currently selected by the campaign scheduler. Other children may retain active tasks while waiting for another scheduling window.",
  absolute_error: "Stop target for the absolute standard error of the combined campaign estimate, after minimum sample requirements are satisfied.",
  relative_error: "Stop target for standard error divided by the magnitude of the combined campaign estimate, after minimum sample requirements are satisfied. The configured value is a fraction, not a percentage.",
};

// Scope definitions to their owning panel: an allocation window is not a
// stratum's observed allocation, and campaign variance shares refer to children.
export function fieldHelp(key = "", label = "", panelId = "") {
  if (["task_summary", "campaign_combined_result"].includes(panelId) && campaignHelp[key]) return campaignHelp[key];
  if (panelId === "campaign_children") {
    if (label === "core-hours") return "Allocated core-hours across this child's sample tasks with publish_result = true, including active tasks and recorded worker CPU accounting. Non-publishing training and setup tasks are excluded.";
    if (label === "coeff") return "Coefficient multiplying this child's contribution to the campaign estimate. Its squared coefficient multiplies the child's variance.";
    if (label === "var ctrb (%)") return "This child's coefficient-squared-weighted variance divided by the total reported campaign variance, summed over the displayed result components. Requires available child uncertainties and a nonzero total variance.";
    if (label === "samples") return "Samples in the child's published result. Earlier non-publishing training stages are excluded here, but count toward the campaign's total sample budget.";
  }
  const stratum = ["stratum_results", "strata"].includes(panelId);
  if (key === "training_projection") return "Projection used to train the sampler and calculate training ESS for these samples. GammaLoop uses real, imag, abs (complex magnitude), or abs_sq (squared magnitude). Vector estimators use a named component or norm (Euclidean norm). Older GammaLoop snapshots without a recorded projection use abs for ESS and are labeled as an unrecorded fallback.";
  if (key.endsWith("_mean_abs")) return "Mean of absolute weighted component values, including zero contributions. Stratified estimates sum conditional absolute means and their independent variances. This is not the absolute value of the signed mean. Older snapshots lacking absolute moments show unavailable.";
  const name = `${key} ${label}`.toLowerCase();
  if (/\brsd\b|_rsd\b/.test(name)) return "RSD = sqrt(N) × SE(estimate) / |estimate|. It uses the signed component and the primary estimator's standard error, including stratified uncertainty where applicable. It stays roughly constant during inference with a fixed proposal. A zero estimate gives infinity for nonzero error; 0/0 is undefined.";
  if (/\bess\b|_ess\b/.test(name)) return "Normalized training ESS = (Σ|w|)² / (N Σw²), where w is the weighted training projection and zero-valued misses count as samples. Values range from 0 to 1. " + (panelId === "stratum_results" ? "This value uses the requested stratum's samples." : "For partitioned sampling the aggregate is Σ(Nₛ × ESSₛ) / ΣNₛ, using each requested stratum separately.") + " Older snapshots without the required training moments show unavailable.";
  if (key === "estimator") return "Pooled estimates use all weighted draws together, including zeros from stratum misses. Stratified estimates sum stratum contributions with errors combined in quadrature. An independent sum combines separate runs with their campaign coefficients. Every reported error belongs to its own estimator's mean.";
  if (key === "uncertainty_status" || label === "Uncertainty") return "Unavailable uncertainty is not zero. Stratified errors require sufficient attempts and matches in every stratum. No observed matches do not establish that a stratum has zero integral.";
  if ((stratum && /probability/.test(name)) || label === "Exact requested p") return "The proposal's probability of requesting this stratum. This differs from its observed allocation and from the chance that a proposed point actually lands in its intended stratum.";
  if (stratum && /allocation/.test(name)) return "Attempted draws assigned to this stratum divided by all attempted draws. This is the observed allocation, not the exact proposal probability.";
  if (stratum && /match fraction|hit rate/.test(name)) return "Matches divided by attempts in the requested stratum. A mismatch contributes zero to the masked estimate and training statistics.";
  if (stratum && /attempts|matches/.test(name)) return /matches/.test(name) ? "Draws whose actual stratum matches their requested stratum." : "All draws requested from this stratum, including mismatches and zero contributions.";
  if (stratum && /variance share|share of.*variance/.test(name)) return "This stratum's reported variance divided by the total reported variance. It requires available stratum errors and a nonzero total variance.";
  if (/impact/.test(name)) return "Magnitude of the largest weighted contribution relative to the magnitude of the estimated sum. For a stratum, the comparison uses that stratum's conditional contribution.";
  if (/max.*weight|weighted value/.test(name)) return "The integrand contribution after applying the returned Jacobian and sampling weights. The corresponding sample point retains the full coordinates and weight factors.";
  if (/compute.*busy|i\/o.*active|compute activity|i\/o activity/.test(name)) return "Time-weighted activity across reporting workers: 100 × total active time / total observed time. Compute and I/O can overlap; these are not CPU/GPU utilization percentages. Queue/database work and waits are I/O; waiting for new work is idle.";
  if (/coverage|report age|measurement window/.test(name)) return "Coverage and report age describe the workers and observations available in the selected measurement window. Missing or stale reports are unavailable, not zero activity.";
  if (/accepted.*\/s|samples\/s|throughput/.test(name)) return "Accepted sample progress per second over the selected measurement window. Retries and unaccepted evaluations are not additional progress.";
  if (/fetch wait|submit wait|queue wait/.test(name)) return "Elapsed waiting time for this operation. Operations can overlap, so individual waits must not be added to infer total utilization.";
  if (/target divisor|pdf divisor/.test(name)) return "Divisor applied to the raw slice values for the selected normalization. The resulting curve describes the displayed slice, not a full-domain integral.";
  if (/slice integral|grid pdf integral/.test(name)) return "Integral over the displayed slice or query grid. It is distinct from a full-domain normalization; grid estimates can miss narrow or unsampled regions.";
  if (/leakage|outside intended cell|mixture mass|target share on slice/.test(name)) return "Mass estimated on the displayed slice using actual stratum membership. Leakage measures proposal mass outside its intended cell. These grid estimates are not full-domain probabilities.";
  if ((key === "count" || label === "Samples") && panelId === "estimate_summary") return "Number of attempted draws in this accumulator, including zero-valued misses. This is the N used in RSD and normalized ESS.";
  if (/effective.source/.test(name)) return "The task or persisted stage from which this effective configuration was resolved. It can differ from the run's original definition.";
  return null;
}
