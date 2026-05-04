export const toFiniteNumber = (value, fallback = 0) => {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
};

export const formatScientific = (value, digits = 6, fallback = "n/a") => {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  if (n === 0) return "0e+0";
  return n.toExponential(digits);
};

export const formatCompactNumber = (value, fallback = "n/a") => {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  if (Number.isInteger(n)) return n.toLocaleString();
  const abs = Math.abs(n);
  if (abs >= 1_000_000 || (abs > 0 && abs < 1e-4)) return formatScientific(n, 4, fallback);
  return n.toLocaleString(undefined, {
    maximumFractionDigits: abs >= 100 ? 2 : 4,
  });
};

export const formatDateTime = (value, fallback = "n/a") => {
  if (!value) return fallback;
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return String(value);
  return dt.toLocaleString();
};

export const formatCentralValueWithError = (value, error, fallback = "n/a") => {
  const central = Number(value);
  if (!Number.isFinite(central)) return fallback;
  const uncertainty = Number(error);
  if (!Number.isFinite(uncertainty) || uncertainty <= 0) {
    return formatScientific(central, 6, fallback);
  }

  const absUncertainty = Math.abs(uncertainty);
  const order = Math.floor(Math.log10(absUncertainty));
  const decimals = Math.max(0, Math.min(12, 1 - order));
  const absCentral = Math.abs(central);

  if (absCentral >= 1_000_000 || (absCentral > 0 && absCentral < 1e-4)) {
    return central.toExponential(Math.max(1, Math.min(12, decimals)));
  }

  return central.toLocaleString(undefined, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
};

export const formatF64Full = (value, fallback = "n/a") => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return numeric.toExponential(16);
};

export const formatEstimateDisplay = (value, error, fallback = "n/a") => {
  const central = Number(value);
  const uncertainty = Number(error);
  const relativePercentValue =
    Number.isFinite(central) && Number.isFinite(uncertainty) && central !== 0
      ? Math.abs(uncertainty / central) * 100
      : null;
  const relativePercentText =
    relativePercentValue != null && Number.isFinite(relativePercentValue)
      ? relativePercentValue >= 1000 || (relativePercentValue > 0 && relativePercentValue < 0.01)
        ? relativePercentValue.toExponential(2)
        : relativePercentValue.toLocaleString(undefined, { maximumFractionDigits: 3 })
      : null;
  const appendRelativeToLatex = (latex) =>
    relativePercentText != null ? `${latex}\\;\\left(${relativePercentText}\\%\\right)` : latex;
  if (!Number.isFinite(central) || !Number.isFinite(uncertainty) || uncertainty < 0) {
    return {
      text: fallback,
      latex: fallback,
      relative_percent: null,
      relative_percent_text: null,
      latex_with_relative: fallback,
    };
  }

  if (uncertainty === 0) {
    const text = formatScientific(central, 6, fallback);
    return {
      text,
      latex: text,
      relative_percent: relativePercentValue,
      relative_percent_text: relativePercentText,
      latex_with_relative: appendRelativeToLatex(text),
    };
  }

  const scaleSource = Math.max(Math.abs(central), Math.abs(uncertainty));
  if (!Number.isFinite(scaleSource) || scaleSource === 0) {
    return {
      text: "(0 ± 0) × 10^0",
      latex: "\\left(0 \\pm 0\\right)\\times 10^{0}",
      relative_percent: relativePercentValue,
      relative_percent_text: relativePercentText,
      latex_with_relative: appendRelativeToLatex("\\left(0 \\pm 0\\right)\\times 10^{0}"),
    };
  }

  const exponent = Math.floor(Math.log10(scaleSource));
  const scale = 10 ** exponent;
  const scaledValue = central / scale;
  const scaledError = Math.abs(uncertainty) / scale;
  const scaledErrorOrder = Math.floor(Math.log10(scaledError));
  const decimals = Math.max(0, Math.min(12, 1 - scaledErrorOrder));
  const valueText = scaledValue.toFixed(decimals);
  const errorText = scaledError.toFixed(decimals);
  return {
    text: `(${valueText} ± ${errorText}) × 10^${exponent}`,
    latex: `\\left(${valueText} \\pm ${errorText}\\right)\\times 10^{${exponent}}`,
    relative_percent: relativePercentValue,
    relative_percent_text: relativePercentText,
    latex_with_relative: appendRelativeToLatex(`\\left(${valueText} \\pm ${errorText}\\right)\\times 10^{${exponent}}`),
  };
};

const TIME_UNIT_SECONDS = {
  s: 1,
  ms: 1e-3,
  "µs": 1e-6,
  ns: 1e-9,
};

const TIME_UNIT_ORDER_SECONDS = [
  { unit: "s", seconds: 1 },
  { unit: "ms", seconds: 1e-3 },
  { unit: "µs", seconds: 1e-6 },
  { unit: "ns", seconds: 1e-9 },
];

export const normalizeTimeUnit = (unit) => {
  const text = String(unit || "").trim().replace(/μ/g, "µ");
  if (!text) return null;
  const lower = text.toLowerCase();
  if (lower === "s") return "s";
  if (lower === "ms") return "ms";
  if (lower === "us" || lower === "µs") return "µs";
  if (lower === "ns") return "ns";
  return null;
};

const unitSeconds = (unit) => {
  const normalized = normalizeTimeUnit(unit);
  if (!normalized) return null;
  return TIME_UNIT_SECONDS[normalized];
};

export const convertTimeValue = (value, fromUnit, toUnit) => {
  const numeric = Number(value);
  const fromSeconds = unitSeconds(fromUnit);
  const toSeconds = unitSeconds(toUnit);
  if (!Number.isFinite(numeric) || fromSeconds == null || toSeconds == null) return null;
  return (numeric * fromSeconds) / toSeconds;
};

export const pickBestTimeUnit = (value, fromUnit) => {
  const numeric = Number(value);
  const fromSeconds = unitSeconds(fromUnit);
  if (!Number.isFinite(numeric) || fromSeconds == null) return null;
  const absoluteSeconds = Math.abs(numeric * fromSeconds);
  for (const candidate of TIME_UNIT_ORDER_SECONDS) {
    if (absoluteSeconds >= candidate.seconds) return candidate.unit;
  }
  return "ns";
};
