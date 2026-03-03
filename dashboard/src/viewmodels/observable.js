import { toFiniteNumber } from "../utils/formatters";

export const deriveObservableMetric = (observable, implementation) => {
  const count = toFiniteNumber(observable?.count, 0);

  if (implementation === "complex") {
    const weightSum = toFiniteNumber(observable?.weight_sum, 0);
    const realSum = toFiniteNumber(observable?.real_sum, 0);
    const imagSum = toFiniteNumber(observable?.imag_sum, 0);
    const normalizer = weightSum > 0 ? weightSum : count > 0 ? count : 0;
    const real = normalizer > 0 ? realSum / normalizer : 0;
    const imag = normalizer > 0 ? imagSum / normalizer : 0;
    return { sampleCount: count, real, imag, mean: real, value: real };
  }

  const sumWeight = toFiniteNumber(observable?.sum_weight, 0);
  const sumSq = toFiniteNumber(observable?.sum_sq, 0);
  const mean = count > 0 ? sumWeight / count : 0;
  const secondMoment = count > 0 ? sumSq / count : 0;
  const variance = Math.max(0, secondMoment - mean * mean);
  const stderr = count > 0 ? Math.sqrt(variance / count) : 0;
  return {
    sampleCount: count,
    mean,
    value: mean,
    stderr,
    lower: mean - stderr,
    upper: mean + stderr,
    spread: 2 * stderr,
  };
};
