import { deriveObservableMetric } from "./observable";

describe("deriveObservableMetric", () => {
  test("derives complex real/imag/abs statistics from serialized fields", () => {
    const metric = deriveObservableMetric(
      {
        count: 2,
        real_sum: 6,
        real_sq_sum: 20,
        imag_sum: 4,
        imag_sq_sum: 10,
        abs_sum: 8,
        abs_sq_sum: 34,
      },
      "complex",
    );

    expect(metric.sampleCount).toBe(2);
    expect(metric.real).toBeCloseTo(3, 10);
    expect(metric.imag).toBeCloseTo(2, 10);
    expect(metric.abs_mean).toBeCloseTo(4, 10);
    expect(metric.real_stderr).toBeCloseTo(Math.sqrt(0.5), 10);
    expect(metric.imag_stderr).toBeCloseTo(Math.sqrt(0.5), 10);
    expect(metric.abs_stderr).toBeCloseTo(Math.sqrt(0.5), 10);
  });

  test("derives scalar mean/abs/stderr from serialized fields", () => {
    const metric = deriveObservableMetric(
      {
        count: 2,
        sum_weighted_value: 6,
        sum_sq: 20,
        sum_abs: 8,
      },
      "scalar",
    );

    expect(metric.sampleCount).toBe(2);
    expect(metric.mean).toBeCloseTo(3, 10);
    expect(metric.meanAbs).toBeCloseTo(4, 10);
    expect(metric.stderr).toBeCloseTo(Math.sqrt(0.5), 10);
  });
});
