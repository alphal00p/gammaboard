import {
  Box,
  Card,
  CardContent,
  FormControl,
  LinearProgress,
  MenuItem,
  Select,
  Typography,
} from "@mui/material";
import prettyMilliseconds from "pretty-ms";
import LatexFormula from "../LatexFormula";
import FigureExportActions, { escapeXml } from "./FigureExportActions";
import {
  convertTimeValue,
  formatCompactNumber,
  formatDateTime,
  formatEstimateDisplay,
  formatF64Full,
  formatScientific,
  normalizeTimeUnit,
  pickBestTimeUnit,
} from "../../utils/formatters";
import { asArray } from "../../utils/collections";
import { isObject } from "./panelView";

const isIsoDateTime = (value) => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value);

const isEstimateValue = (value) =>
  isObject(value) &&
  value.kind === "estimate" &&
  Number.isFinite(Number(value.value)) &&
  Number.isFinite(Number(value.error));

const isTargetComparisonValue = (value) =>
  isObject(value) &&
  value.kind === "target_comparison" &&
  Number.isFinite(Number(value.value)) &&
  Number.isFinite(Number(value.error)) &&
  Number.isFinite(Number(value.target));

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
  const kind = typeof value.kind === "string" ? value.kind : "domain";
  if (kind === "continuous" && Number.isFinite(Number(value.dimension))) {
    return `continuous (${Number(value.dimension)}D)`;
  }
  if (kind === "discrete") return `discrete (${asArray(value.branches).length} branches)`;
  return kind;
};

const formatDebugJson = (value) => {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
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

export const ProgressPanel = ({ title, state }) => {
  const current = Number(state?.current);
  const total = Number(state?.total);
  const eta = formatEtaSeconds(state?.eta_seconds);
  const progress = Number.isFinite(current) && Number.isFinite(total) && total > 0 ? (current / total) * 100 : 0;
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
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
      </CardContent>
    </Card>
  );
};

export const KeyValuePanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 2 }}>
        {title}
      </Typography>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "minmax(0, 1fr)", lg: "minmax(0, 1fr) minmax(0, 1fr)" },
          gap: 1.5,
        }}
      >
        {asArray(state?.entries).map((entry) => {
          const displayEntry = withDynamicTimeUnitEntry(entry);
          return (
            <Box
              key={displayEntry.key}
              sx={{
                display: "grid",
                gridTemplateColumns: "minmax(120px, 0.9fr) minmax(0, 1.1fr)",
                gap: 1,
                py: 0.5,
                borderBottom: "1px solid",
                borderColor: "divider",
              }}
            >
              <Typography variant="body2" color="text.secondary">
                {displayEntry.label}
              </Typography>
              {isEstimateValue(displayEntry.value) ? (
                <EstimateValueBlock value={displayEntry.value} />
              ) : isTargetComparisonValue(displayEntry.value) ? (
                <TargetComparisonValueBlock value={displayEntry.value} />
              ) : displayEntry?.key === "domain" && isObject(displayEntry.value) ? (
                <Box sx={{ minWidth: 0 }}>
                  <Box component="details">
                    <Box component="summary" sx={{ cursor: "pointer", fontSize: "0.8rem", color: "text.secondary" }}>
                      {domainSummaryText(displayEntry.value)}
                    </Box>
                    <Typography
                      variant="caption"
                      sx={{
                        mt: 0.5,
                        display: "block",
                        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                        color: "text.primary",
                      }}
                    >
                      {formatDebugJson(displayEntry.value)}
                    </Typography>
                  </Box>
                </Box>
              ) : (
                <Typography
                  variant="body2"
                  sx={{
                    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                    wordBreak: "break-word",
                    whiteSpace: "pre-wrap",
                    color:
                      displayEntry.tone === "good"
                        ? "success.main"
                        : displayEntry.tone === "warning"
                          ? "warning.main"
                          : displayEntry.tone === "critical"
                            ? "error.main"
                            : "text.primary",
                  }}
                >
                  {renderStructuredValue(displayEntry.value)}
                </Typography>
              )}
            </Box>
          );
        })}
      </Box>
    </CardContent>
  </Card>
);

const EstimateValueBlock = ({ value }) => {
  const central = Number(value?.value);
  const error = Number(value?.error);
  const estimate = formatEstimateDisplay(central, error, "n/a");
  return (
    <Box sx={{ minWidth: 0 }}>
      <Box sx={{ fontSize: "1.05rem", fontWeight: 700, lineHeight: 1.35, whiteSpace: "nowrap", overflowX: "auto", pb: 0.25 }}>
        <LatexFormula latex={estimate.latex_with_relative || estimate.latex} display={false} fallbackPrefix="Estimate" />
      </Box>
      <Box component="details" sx={{ mt: 0.5 }}>
        <Box component="summary" sx={{ cursor: "pointer", fontSize: "0.8rem", color: "text.secondary" }}>
          Full precision (f64)
        </Box>
        <Typography variant="caption" sx={{ mt: 0.5, display: "block", fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace", whiteSpace: "pre-wrap" }}>
          {`value = ${formatF64Full(central, "n/a")}\nerror = ${formatF64Full(error, "n/a")}`}
        </Typography>
      </Box>
    </Box>
  );
};

const TargetComparisonValueBlock = ({ value }) => {
  const central = Number(value?.value);
  const error = Number(value?.error);
  const target = Number(value?.target);
  const deltaPercent = Number(value?.delta_percent);
  const deltaSigma = Number(value?.delta_sigma);
  const targetText = formatScientific(target, 4, "n/a");
  const deltaPercentText = Number.isFinite(deltaPercent) ? formatScientific(deltaPercent, 4, "n/a") : "n/a";
  const deltaSigmaText = Number.isFinite(deltaSigma) ? formatScientific(deltaSigma, 4, "n/a") : "n/a";
  return (
    <Box sx={{ minWidth: 0 }}>
      <Box sx={{ fontSize: "1rem", fontWeight: 650, lineHeight: 1.4, pb: 0.1 }}>
        <LatexFormula latex={`t=${targetText}`} display={false} fallbackPrefix="Target" />
      </Box>
      <Box sx={{ fontSize: "0.95rem", lineHeight: 1.35, color: "text.secondary", pb: 0.15 }}>
        <LatexFormula latex={`\\Delta=${deltaPercentText}\\%,\\;${deltaSigmaText}\\sigma`} display={false} fallbackPrefix="Delta" />
      </Box>
      <Box component="details" sx={{ mt: 0.5 }}>
        <Box component="summary" sx={{ cursor: "pointer", fontSize: "0.8rem", color: "text.secondary" }}>
          Full precision (f64)
        </Box>
        <Typography variant="caption" sx={{ mt: 0.5, display: "block", fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace", whiteSpace: "pre-wrap" }}>
          {`value = ${formatF64Full(central, "n/a")}\nerror = ${formatF64Full(error, "n/a")}\ntarget = ${formatF64Full(target, "n/a")}\ndelta_percent = ${formatF64Full(deltaPercent, "n/a")}\ndelta_sigma = ${formatF64Full(deltaSigma, "n/a")}`}
        </Typography>
      </Box>
    </Box>
  );
};

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
          <Box sx={{ border: "1px solid", borderColor: "divider", borderRadius: 1.5, p: 1, overflowX: "auto", backgroundColor: "background.paper" }}>
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
          `<text x="16" y="${y - 1}" font-size="12" font-family="monospace" fill="#334155">${escapeXml(segment.label || segment.key)}</text>`,
          `<text x="${width}" y="${y - 1}" text-anchor="end" font-size="12" font-family="monospace" fill="#475569">${escapeXml(`${formatScientific(displayValueForSegment(segment), 4)} ${displayTimeUnit} (${formatScientific(percent, 3)}%)`)}</text>`,
        ].join("");
      })
      .join("");
    return [
      `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`,
      `<text x="0" y="60" font-size="12" font-family="monospace" fill="#64748b">total ${escapeXml(formatScientific(displayTotal, 4))} ${displayTimeUnit}</text>`,
      bars,
      legend,
      "</svg>",
    ].join("");
  };

  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
            <FigureExportActions baseName={state?.panel_id || title || "tick_breakdown"} payload={{ panel_id: state?.panel_id ?? null, kind: "tick_breakdown", state }} svgBuilder={buildTickBreakdownSvg} />
            <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
              total {formatScientific(displayTotal, 4)} {displayTimeUnit}
            </Typography>
          </Box>
        </Box>
        <Box sx={{ display: "flex", width: "100%", minHeight: 44, borderRadius: 1.5, overflow: "hidden", border: "1px solid", borderColor: "divider", backgroundColor: "rgba(15,23,42,0.04)" }}>
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

export const SelectPanel = ({ title, descriptor, value, onValueChange }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 2 }}>
        {title}
      </Typography>
      <FormControl fullWidth size="small">
        <Select value={value ?? ""} onChange={(event) => onValueChange?.(descriptor.panel_id, event.target.value)}>
          {asArray(descriptor?.state?.options).map((option) => (
            <MenuItem key={String(option.value)} value={option.value}>
              {option.label}
            </MenuItem>
          ))}
        </Select>
      </FormControl>
    </CardContent>
  </Card>
);
