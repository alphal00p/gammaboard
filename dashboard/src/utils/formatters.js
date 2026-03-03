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

export const formatDateTime = (value, fallback = "n/a") => {
  if (!value) return fallback;
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return String(value);
  return dt.toLocaleString();
};
