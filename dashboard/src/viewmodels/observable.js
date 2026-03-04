import { toFiniteNumber } from "../utils/formatters";

const deriveScalarStats = (sum, sumSq, count) => {
  const mean = count > 0 ? sum / count : 0;
  const secondMoment = count > 0 ? sumSq / count : 0;
  const variance = Math.max(0, secondMoment - mean * mean);
  const stderr = count > 0 ? Math.sqrt(variance / count) : 0;
  return {
    mean,
    stderr,
    lower: mean - stderr,
    upper: mean + stderr,
    spread: 2 * stderr,
  };
};

export const deriveObservableMetric = (observable, implementation) => {
  const count = toFiniteNumber(observable?.count, 0);

  if (implementation === "complex") {
    const realSum = toFiniteNumber(observable?.real_sum, 0);
    const imagSum = toFiniteNumber(observable?.imag_sum, 0);
    const absSum = toFiniteNumber(observable?.abs_sum, 0);
    const absSqSum = toFiniteNumber(observable?.abs_sq_sum, 0);
    const realSqSum = toFiniteNumber(observable?.real_sq_sum, 0);
    const imagSqSum = toFiniteNumber(observable?.imag_sq_sum, 0);
    const real = deriveScalarStats(realSum, realSqSum, count);
    const imag = deriveScalarStats(imagSum, imagSqSum, count);
    const abs = deriveScalarStats(absSum, absSqSum, count);
    return {
      sampleCount: count,
      real: real.mean,
      imag: imag.mean,
      abs_mean: abs.mean,
      abs_stderr: abs.stderr,
      abs_lower: abs.lower,
      abs_upper: abs.upper,
      abs_spread: abs.spread,
      real_stderr: real.stderr,
      imag_stderr: imag.stderr,
      real_lower: real.lower,
      real_upper: real.upper,
      real_spread: real.spread,
      imag_lower: imag.lower,
      imag_upper: imag.upper,
      imag_spread: imag.spread,
      mean: real.mean,
      value: real.mean,
    };
  }

  const sumWeight = toFiniteNumber(observable?.sum_weight, 0);
  const sumSq = toFiniteNumber(observable?.sum_sq, 0);
  const sumAbs = toFiniteNumber(observable?.sum_abs, 0);
  const scalar = deriveScalarStats(sumWeight, sumSq, count);
  const meanAbs = count > 0 ? sumAbs / count : 0;
  return {
    sampleCount: count,
    mean: scalar.mean,
    value: scalar.mean,
    meanAbs,
    stderr: scalar.stderr,
    lower: scalar.lower,
    upper: scalar.upper,
    spread: scalar.spread,
  };
};
