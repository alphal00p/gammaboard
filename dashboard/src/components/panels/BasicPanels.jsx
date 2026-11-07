import {
  Box,
  Card,
  CardContent,
  FormControl,
  LinearProgress,
  MenuItem,
  Select,
  Stack,
  Typography,
  useTheme,
} from "@mui/material";
import prettyMilliseconds from "pretty-ms";
import FigureExportActions, { escapeXml } from "./FigureExportActions";
import {
  convertTimeValue,
  formatCompactNumber,
  formatDateTime,
  formatEstimateDisplay,
  formatDecimalSignificant,
  formatF64Full,
  formatScientific,
  normalizeTimeUnit,
  pickBestTimeUnit,
} from "../../utils/formatters";
import { asArray } from "../../utils/collections";
import { isObject } from "./panelView";
import StatusChip from "../common/StatusChip";
import DisclosureField from "../common/DisclosureField";
import ConfigurationTree from "./ConfigurationTree";
import { fieldHelp } from "./fieldHelp";
import { PanelHeader } from "../common/PanelChrome";

const isIsoDateTime = (value) => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value);

const isEstimateValue = (value) =>
  isObject(value) &&
  value.kind === "estimate" &&
  (value.value == null || Number.isFinite(Number(value.value))) &&
  (value.error == null || (Number.isFinite(Number(value.error)) && Number(value.error) >= 0));

const isTargetComparisonValue = (value) =>
  isObject(value) &&
  value.kind === "target_comparison" &&
  Number.isFinite(Number(value.value)) &&
  (value.error == null || Number.isFinite(Number(value.error))) &&
  value.target != null && Number.isFinite(Number(value.target));

export const renderStructuredValue = (value) => {
  if (value == null) return "none";
  if (typeof value === "number") return formatCompactNumber(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "string") {
    if (!value.trim()) return "none";
    if (isIsoDateTime(value)) return formatDateTime(value, value);
    return value;
  }
  return JSON.stringify(value);
};

const TIME_UNIT_SUFFIX_RE = /^(?<base>.+?)\s+\((?<unit>[^)]+)\)$/;
const TIME_UNIT_PER_RE = /^(?<base>.+?)\s+(?<unit>[a-zA-Zµμ]+)\s*\/\s*(?<per>.+)$/;
const SEC_WORD_RE = /\bSec\b/g;

const normalizeTimeLabelOnly = (label) => {
  const normalizedSec = label.replace(SEC_WORD_RE, "s");
  const perMatch = TIME_UNIT_PER_RE.exec(normalizedSec);
  if (!perMatch?.groups) return normalizedSec;
  const normalizedUnit = normalizeTimeUnit(perMatch.groups.unit);
  if (!normalizedUnit) return normalizedSec;
  return `${perMatch.groups.base} ${normalizedUnit} / ${perMatch.groups.per}`;
};

const withDynamicTimeUnitEntry = (entry) => {
  const label = typeof entry?.label === "string" ? entry.label.trim() : "";
  if (!label) return entry;
  const normalizedLabel = normalizeTimeLabelOnly(label);
  if (typeof entry?.value !== "number" || !Number.isFinite(entry.value)) {
    return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
  }
  const parsed = TIME_UNIT_SUFFIX_RE.exec(label);
  if (parsed?.groups) {
    const baseUnit = normalizeTimeUnit(parsed.groups.unit);
    if (!baseUnit) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
    const displayUnit = pickBestTimeUnit(entry.value, baseUnit);
    if (!displayUnit) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
    const converted = convertTimeValue(entry.value, baseUnit, displayUnit);
    if (!Number.isFinite(converted)) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
    return { ...entry, label: `${parsed.groups.base} (${displayUnit})`, value: converted };
  }

  const per = TIME_UNIT_PER_RE.exec(label);
  if (!per?.groups) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
  const baseUnit = normalizeTimeUnit(per.groups.unit);
  if (!baseUnit) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
  const displayUnit = pickBestTimeUnit(entry.value, baseUnit);
  if (!displayUnit) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
  const converted = convertTimeValue(entry.value, baseUnit, displayUnit);
  if (!Number.isFinite(converted)) return normalizedLabel === label ? entry : { ...entry, label: normalizedLabel };
  return { ...entry, label: `${per.groups.base} ${displayUnit} / ${per.groups.per}`, value: converted };
};

const domainSummaryText = (value) => {
  if (!isObject(value)) return "Expand";
  if (value.rectangular) {
    const { strata_dims = 0, discrete_cardinalities = [], continuous_dims } = value.rectangular;
    return `Strata [${discrete_cardinalities.slice(0, strata_dims).join(", ")}]; integrand discrete [${discrete_cardinalities.slice(strata_dims).join(", ")}], continuous ${continuous_dims}D`;
  }
  if (value.discrete) {
    const { axis_role = "integrand", branches = [] } = value.discrete;
    return `${axis_role === "stratum" ? "Stratum" : "Integrand"} branch: ${branches.map((b) => b.index).join(", ")}`;
  }
  return "Domain";
};

const formatEtaSeconds = (seconds) => {
  const numericSeconds = Number(seconds);
  if (!Number.isFinite(numericSeconds) || numericSeconds < 0) return null;
  const etaMs = Math.max(0, Math.round(numericSeconds * 1000));
  return prettyMilliseconds(etaMs, {
    colonNotation: true,
    secondsDecimalDigits: 0,
    keepDecimalsOnWholeSeconds: false,
    verbose: false,
  });
};

export const ProgressPanel = ({ title, state, controls = null }) => {
  const current = Number(state?.current);
  const total = Number(state?.total);
  const eta = formatEtaSeconds(state?.eta_seconds);
  const progress = Number.isFinite(current) && Number.isFinite(total) && total > 0 ? (current / total) * 100 : 0;
  if (state?.panel_id === "sample_progress") return <Stack direction="row" spacing={1.5} alignItems="center" sx={{ py: 0.5 }}>
    <LinearProgress aria-label={title || "Sample progress"}
      variant={Number.isFinite(total) && total > 0 ? "determinate" : "indeterminate"}
      value={Math.max(0, Math.min(100, progress))} sx={{ flex: 1, height: 6, borderRadius: 999 }} />
    {eta && <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "nowrap" }}>ETA: {eta}</Typography>}
  </Stack>;
  if (state?.panel_id === "campaign_progress") return <Card variant="outlined"><CardContent>
    <Stack direction="row" justifyContent="space-between" alignItems="baseline" spacing={2} sx={{ mb: 1 }}>
      <Typography variant="subtitle1">Campaign progress</Typography>
      <Typography variant="subtitle1" sx={{ textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
        {Number.isFinite(current) ? current.toLocaleString() : "0"}
        {state.total != null && Number.isFinite(total) ? ` / ${total.toLocaleString()}` : ""} samples
      </Typography>
    </Stack>
    <LinearProgress aria-label="Campaign progress" variant={Number.isFinite(total) && total > 0 ? "determinate" : "indeterminate"}
      value={Math.max(0, Math.min(100, progress))} sx={{ height: 6, borderRadius: 999 }} />
    {eta && <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 1 }}>ETA: {eta}</Typography>}
  </CardContent></Card>;
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={2} alignItems={{ sm: "center" }}>
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography variant="h5" sx={{ fontFamily: "monospace", mb: 1 }}>
              {Number.isFinite(current) ? current.toLocaleString() : "0"}
              {Number.isFinite(total) ? ` / ${total.toLocaleString()}` : ""}
            </Typography>
            <LinearProgress
              variant={Number.isFinite(total) && total > 0 ? "determinate" : "indeterminate"}
              value={progress}
              sx={{ height: 6, borderRadius: 999 }}
            />
            {eta ? (
              <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
                ETA: {eta}
              </Typography>
            ) : null}
          </Box>
          {controls}
        </Stack>
      </CardContent>
    </Card>
  );
};

const RESULT_NOTES = new Set(["diagnostics_basis", "uncertainty_note"]);
const RESULT_PANELS = new Set(["estimate_summary", "campaign_combined_result"]);

export function ScientificValue({ text }) {
  const match = /^(.*?)(?: × 10\^|e)([+-]?\d+)$/.exec(text);
  if (!match) return <>{text}</>;
  return <>{match[1]}{Number(match[2]) !== 0 && <> × 10<sup>{Number(match[2])}</sup></>}</>;
}

function PrecisionText({ children }) {
  return <Typography component="pre" variant="body2" sx={{ fontFamily: "monospace" }}>{children}</Typography>;
}

export const EstimateValueBlock = ({ value, label, help }) => {
  const central = value?.value == null ? null : Number(value.value);
  const error = value?.error == null ? null : Number(value.error);
  const estimator = value?.estimator || "pooled";
  const estimate = formatEstimateDisplay(central, error, "unavailable");
  const details = [`estimator = ${estimator}`, `mean = ${formatF64Full(central, "unavailable")}`];
  if (estimator === "stratified") {
    details.push(`strat_err = ${formatF64Full(error, "unavailable: insufficient stratum coverage")}`);
    if (value?.basic) details.push(`basic_mean = ${formatF64Full(value.basic.mean, "unavailable")}`, `basic_err = ${formatF64Full(value.basic.error, "unavailable")}`);
  } else {
    details.push(`basic_err = ${formatF64Full(error, "unavailable: fewer than two samples")}`, "strat_err = not used");
  }
  const compact = <Box component="span" sx={{ display: "inline-flex", flexWrap: "wrap", alignItems: "baseline", columnGap: 0.5 }}>
    <span>{central == null ? "Mean unavailable" : <ScientificValue text={estimate.text} />}</span>
    {central != null && error == null ? <span>· error unavailable</span> : estimate.relative_percent_text != null && <span>({estimate.relative_percent_text}%)</span>}
  </Box>;
  return <DisclosureField label={label || "Estimate details"} value={compact} inline={!label}>
    <PrecisionText>{details.join("\n")}</PrecisionText>
    <Typography variant="caption" component="p" sx={{ mt: 1 }}>Parenthesized digits give the uncertainty in the last displayed digits; for example, 5.4123(25) means 5.4123 ± 0.0025.</Typography>
    {value?.basic && <Typography variant="caption" component="p">The pooled reference error belongs to its own pooled mean.</Typography>}
    {help && <Typography variant="caption" component="p" sx={{ mt: 1 }}>{help}</Typography>}
  </DisclosureField>;
};

const TargetComparisonValueBlock = ({ value, label }) => {
  const signed = (number) => number == null || !Number.isFinite(Number(number)) ? "n/a"
    : `${Number(number) > 0 ? "+" : ""}${formatDecimalSignificant(number)}`;
  return <DisclosureField label={label || "Target comparison details"} inline={!label}
    value={<Box component="span" sx={{ display: "inline-flex", alignItems: "baseline", flexWrap: "wrap", columnGap: 0.75 }}>
      <span><ScientificValue text={formatScientific(value.target, 4)} /></span>
      <span>· Δ {signed(value.delta_percent)}%</span><span>· {signed(value.delta_sigma)}σ</span>
    </Box>}>
    <PrecisionText>{`estimator = ${value.estimator || "pooled"}\nvalue = ${formatF64Full(value.value)}\nerror = ${formatF64Full(value.error)}\ntarget = ${formatF64Full(value.target)}\ndelta_percent = ${formatF64Full(value.delta_percent)}\ndelta_sigma = ${formatF64Full(value.delta_sigma)}`}</PrecisionText>
    <Typography variant="caption" component="p" sx={{ mt: 1 }}>Delta is 100 × (estimate − target) / |target|. Pull is (estimate − target) / standard error. Positive values lie above the target. Undefined comparisons remain unavailable.</Typography>
  </DisclosureField>;
};

function ValueField({ entry, state }) {
  const display = withDynamicTimeUnitEntry(entry);
  const { key, label, value } = display;
  const help = entry.help || fieldHelp(key, label, state.panel_id);
  const errorNote = asArray(state.entries).find((e) => e.key === "uncertainty_note")?.value;
  if (isEstimateValue(value)) return <EstimateValueBlock label={label} value={value} help={[help, errorNote].filter(Boolean).join(" ")} />;
  if (isTargetComparisonValue(value)) return <TargetComparisonValueBlock label={label} value={value} />;
  if (isObject(value) || Array.isArray(value)) return <DisclosureField label={label}
    value={key === "domain" ? domainSummaryText(value) : `${Object.keys(value).length} ${Array.isArray(value) ? "items" : "fields"}`}>
    <ConfigurationTree value={value} />
  </DisclosureField>;
  const precise = typeof value === "number" && !Number.isInteger(value);
  return <DisclosureField label={label} value={["state", "status"].includes(label?.toLowerCase()) && typeof value === "string"
    ? <StatusChip status={value} /> : <Typography component="span" variant="body2" sx={{ overflowWrap: "anywhere",
      color: display.tone === "good" ? "success.main" : display.tone === "warning" ? "warning.main" : display.tone === "critical" ? "error.main" : "inherit" }}>
      {renderStructuredValue(value)}</Typography>}>
    {help || precise ? <>
      {help && <Typography variant="body2" sx={{ mb: precise ? 1 : 0 }}>{help}</Typography>}
      {precise && <PrecisionText>{formatF64Full(value)}</PrecisionText>}
    </> : null}
  </DisclosureField>;
}

export const KeyValuePanel = ({ title, state }) => {
  const result = RESULT_PANELS.has(state?.panel_id);
  const entries = asArray(state?.entries).filter((entry) => !result || !RESULT_NOTES.has(entry.key));
  const allEstimates = entries.filter((entry) => isEstimateValue(entry.value));
  const estimates = result ? allEstimates : [];
  const grouped = new Set();
  const hasTargets = entries.some((entry) => isTargetComparisonValue(entry.value));
  const groups = estimates.map((mean) => {
    const component = /^component_(.*)_mean$/.exec(mean.key)?.[1] ?? /^estimate_(.*)$/.exec(mean.key)?.[1] ?? mean.key.replace(/_mean$/, "");
    const target = entries.find((entry) => entry.key === `target_comparison_${component}`);
    const rsd = entries.find((entry) => entry.key === `component_${component}_rsd`);
    const fields = [mean, target || (hasTargets ? { key: `target_comparison_${component}`, label: `${component} vs Target`, value: "not set" } : null), rsd].filter(Boolean);
    fields.forEach((entry) => grouped.add(entry.key));
    return { component, fields };
  });
  const common = entries.filter((entry) => !grouped.has(entry.key));
  const renderFields = (fields) => fields.map((entry) => <Box key={entry.key} sx={{ minWidth: 0, borderBottom: 1, borderColor: "divider" }}><ValueField entry={entry} state={state} /></Box>);
  return <Card variant="outlined"><CardContent>
    {(title || allEstimates.length > 0) && <PanelHeader title={title}>
      {allEstimates.length > 0 && <FigureExportActions baseName={state?.panel_id || title} payload={{ kind: "key_value", state }} imageFormat={null} />}
    </PanelHeader>}
    {state?.panel_id === "max_weight_summary" ? <Box sx={{ display: "grid", gridTemplateColumns: { xs: "minmax(0, 1fr)", md: "repeat(3, minmax(0, 1fr))" }, gap: 2 }}>
      {renderFields(entries.filter((entry) => entry.key !== "weight_basis").map((entry) => ({ ...entry,
        help: [fieldHelp(entry.key, entry.label, state.panel_id), entries.find((field) => field.key === "weight_basis")?.value].filter(Boolean).join(" "),
      })))}
    </Box> : state?.panel_id?.endsWith("_config") ? <ConfigurationTree entries={asArray(state.entries)} /> : <>
      <Box sx={{ display: "grid", gridTemplateColumns: { xs: "minmax(0, 1fr)", lg: "repeat(2, minmax(0, 1fr))" }, columnGap: 2 }}>
        {renderFields(common.filter((entry) => ["count", "estimator"].includes(entry.key)))}
      </Box>
      {groups.length > 0 && <Box sx={{ display: "grid", gridTemplateColumns: { xs: "minmax(0, 1fr)", lg: "repeat(2, minmax(0, 1fr))" }, columnGap: 2 }}>
        {groups.map(({ component, fields }) => <Box key={component} data-result-component={component} sx={{ minWidth: 0 }}>{renderFields(fields)}</Box>)}
      </Box>}
      <Box sx={{ display: "grid", gridTemplateColumns: { xs: "minmax(0, 1fr)", lg: "repeat(2, minmax(0, 1fr))" }, columnGap: 2 }}>
        {renderFields(common.filter((entry) => !["count", "estimator"].includes(entry.key)))}
      </Box>
    </>}
  </CardContent></Card>;
};

export const EmptyPlotPanel = ({ title }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 1 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary">
        No plot data available.
      </Typography>
    </CardContent>
  </Card>
);

export const TextPanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 1 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
        {renderStructuredValue(state?.text)}
      </Typography>
    </CardContent>
  </Card>
);

export const SvgPanel = ({ title, state }) => {
  const svg = typeof state?.svg === "string" ? state.svg.trim() : "";
  const message = typeof state?.message === "string" ? state.message.trim() : "";
  const src = svg ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}` : null;
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 1 }}>
          {title}
        </Typography>
        {src ? (
          <Box sx={{ border: "1px solid", borderColor: "divider", borderRadius: 1.5, p: 1, overflowX: "auto", backgroundColor: "#fff" }}>
            <Box component="img" src={src} alt={title} sx={{ display: "block", width: "100%", height: "auto", minWidth: 320 }} />
          </Box>
        ) : (
          <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
            {message || "No SVG payload available."}
          </Typography>
        )}
      </CardContent>
    </Card>
  );
};

export const TickBreakdownPanel = ({ title, state }) => {
  const { palette } = useTheme();
  const totalMs = Number(state?.total_ms);
  const segments = asArray(state?.segments)
    .map((segment) => ({ ...segment, valueMs: Number(segment?.value_ms) }))
    .filter((segment) => Number.isFinite(segment.valueMs) && segment.valueMs > 0);
  const normalizedTotal =
    Number.isFinite(totalMs) && totalMs > 0 ? totalMs : segments.reduce((sum, segment) => sum + segment.valueMs, 0);
  const displayTimeUnit = pickBestTimeUnit(normalizedTotal, "ms") || "ms";
  const displayTotal = convertTimeValue(normalizedTotal, "ms", displayTimeUnit) ?? normalizedTotal;
  const displayValueForSegment = (segment) => convertTimeValue(segment.valueMs, "ms", displayTimeUnit) ?? segment.valueMs;

  if (segments.length === 0 || !Number.isFinite(normalizedTotal) || normalizedTotal <= 0) return null;
  const buildTickBreakdownSvg = () => {
    const width = 960;
    const barHeight = 36;
    const rowHeight = 22;
    const legendTop = 64;
    const height = legendTop + segments.length * rowHeight + 8;
    let offsetX = 0;
    const bars = segments
      .map((segment) => {
        const percent = (segment.valueMs / normalizedTotal) * 100;
        const segmentWidth = Math.max((width * percent) / 100, 2);
        const rect = `<rect x="${offsetX}" y="8" width="${segmentWidth}" height="${barHeight}" fill="${escapeXml(segment.color || "#0f766e")}" />`;
        offsetX += segmentWidth;
        return rect;
      })
      .join("");
    const legend = segments
      .map((segment, index) => {
        const percent = (segment.valueMs / normalizedTotal) * 100;
        const y = legendTop + index * rowHeight;
        return [
          `<rect x="0" y="${y - 10}" width="10" height="10" fill="${escapeXml(segment.color || "#0f766e")}" />`,
          `<text x="16" y="${y - 1}" font-size="12" font-family="monospace" fill="${palette.text.primary}">${escapeXml(segment.label || segment.key)}</text>`,
          `<text x="${width}" y="${y - 1}" text-anchor="end" font-size="12" font-family="monospace" fill="${palette.text.secondary}">${escapeXml(`${formatScientific(displayValueForSegment(segment), 4)} ${displayTimeUnit} (${formatScientific(percent, 3)}%)`)}</text>`,
        ].join("");
      })
      .join("");
    return [
      `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`,
      `<rect width="100%" height="100%" fill="${palette.background.paper}" />`,
      `<text x="0" y="60" font-size="12" font-family="monospace" fill="${palette.text.secondary}">total ${escapeXml(formatScientific(displayTotal, 4))} ${displayTimeUnit}</text>`,
      bars,
      legend,
      "</svg>",
    ].join("");
  };

  return (
    <Card variant="outlined">
      <CardContent>
        <PanelHeader title={title}>
          <FigureExportActions baseName={state?.panel_id || title || "tick_breakdown"} payload={{ panel_id: state?.panel_id ?? null, kind: "tick_breakdown", state }} svgBuilder={buildTickBreakdownSvg} />
          <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
            total {formatScientific(displayTotal, 4)} {displayTimeUnit}
          </Typography>
        </PanelHeader>
        <Box sx={{ display: "flex", width: "100%", minHeight: 44, borderRadius: 1.5, overflow: "hidden", border: "1px solid", borderColor: "divider", backgroundColor: "action.hover" }}>
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            const showInlineLabel = percent >= 10;
            return (
              <Box key={segment.key} title={`${segment.label}: ${formatScientific(displayValueForSegment(segment), 4)} ${displayTimeUnit} (${formatScientific(percent, 3)}%)`} sx={{ width: `${Math.max(percent, 1.5)}%`, minWidth: 0, px: showInlineLabel ? 1 : 0, py: 0.75, display: "flex", alignItems: "center", justifyContent: showInlineLabel ? "space-between" : "center", gap: 1, color: "#fff", backgroundColor: segment.color || "#0f766e" }}>
                {showInlineLabel ? (
                  <>
                    <Typography variant="caption" sx={{ fontWeight: 600, color: "inherit", lineHeight: 1.15 }}>
                      {segment.label}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "inherit", opacity: 0.95, lineHeight: 1.15 }}>
                      {formatScientific(displayValueForSegment(segment), 3)} {displayTimeUnit}
                    </Typography>
                  </>
                ) : null}
              </Box>
            );
          })}
        </Box>
        <Box sx={{ mt: 1.25, display: "grid", gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" }, gap: 0.75 }}>
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            return (
              <Box key={`${segment.key}-legend`} sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Box sx={{ width: 10, height: 10, borderRadius: 0.5, backgroundColor: segment.color || "#0f766e", flexShrink: 0 }} />
                <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0 }}>
                  {segment.label}
                </Typography>
                <Typography variant="caption" sx={{ ml: "auto", fontFamily: "monospace", color: "text.secondary", whiteSpace: "nowrap" }}>
                  {formatScientific(displayValueForSegment(segment), 4)} {displayTimeUnit} ({formatScientific(percent, 3)}%)
                </Typography>
              </Box>
            );
          })}
        </Box>
      </CardContent>
    </Card>
  );
};

export const SelectInput = ({ title, descriptor, value, onValueChange }) => (
  <FormControl fullWidth size="small">
    <Select inputProps={{ "aria-label": title }} value={value ?? descriptor?.state?.default_value ?? ""}
      onChange={(event) => onValueChange?.(descriptor.panel_id, event.target.value)}>
      {asArray(descriptor?.state?.options).map((option) => (
        <MenuItem key={String(option.value)} value={option.value}>{option.label}</MenuItem>
      ))}
    </Select>
  </FormControl>
);

export const SelectPanel = ({ title, ...props }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 2 }}>
        {title}
      </Typography>
      <SelectInput title={title} {...props} />
    </CardContent>
  </Card>
);
