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
