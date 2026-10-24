import { numberOrNaN } from '../../utils/formatters';
export const STRATUM_COLORS = [
  '#0072b2',
  '#d55e00',
  '#009e73',
  '#cc79a7',
  '#b58b00',
  '#56b4e9',
  '#8c564b',
  '#6f63ad'
];
export const stratumColor = (index) => STRATUM_COLORS[index % STRATUM_COLORS.length];
export const prefixLabel = (prefix) => `(${prefix.join(',')})`;
export const finite = (value) => Number.isFinite(numberOrNaN(value));
export const fraction = (a, b) => (finite(a) && finite(b) && b > 0 ? a / b : null);
export const varianceShares = (rows, component) => {
  const errors = rows.map((row) => row.estimates?.[component]?.error);
  if (!errors.length || errors.some((error) => !finite(error) || error < 0))
    return rows.map(() => null);
  const scale = Math.max(...errors);
  if (!scale) return rows.map(() => null);
  const variances = errors.map((error) => (error / scale) ** 2);
  const total = variances.reduce((a, b) => a + b, 0);
  return variances.map((v) => v / total);
};

// Differences use adjacent retained snapshots: this is a reporting interval, not
// a fixed-size training window (history may be downsampled).
export const stratumHistory = (state, component, metric, interval = false) => {
  const frames = state.frames || [];
  const latest = frames.at(-1);
  return (latest?.rows || []).map((row, k) => ({
    name: prefixLabel(row.prefix),
    color: stratumColor(k),
    points: frames.map((frame, i) => {
      const current = frame.rows[k];
      const previous = interval && i ? frames[i - 1].rows[k] : null;
      const attempts = current.attempts - (previous?.attempts || 0);
      const matches = current.matches - (previous?.matches || 0);
      const samples = frame.samples - (previous ? frames[i - 1].samples : 0);
      const estimate = current.estimates?.[component];
      const y =
        metric === 'allocation'
          ? fraction(attempts, samples)
          : metric === 'hit_rate'
            ? fraction(matches, attempts)
            : estimate?.mean;
      return {
        x: frame.samples,
        wall_time_ms: frame.wall_time_ms,
        sampler_uptime_ms: frame.sampler_uptime_ms,
        y: finite(y) ? y : null,
        error: metric === 'contribution' ? (estimate?.error ?? null) : null
      };
    })
  }));
};
