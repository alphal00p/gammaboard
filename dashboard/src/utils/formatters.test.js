import { expect, test } from "vitest";
import { formatComparisonDecimal, formatDecimalSignificant, formatEstimateDisplay } from "./formatters";

test.each([
  [5.4123, 0.0025, "5.4123(25)"],
  [1.5, 0.5, "1.50(50)"],  [-1.318e-4, 0.0039e-4, "-1.3180(39) × 10^-4"],
  [1.23e-3, 0.20e-3, "1.23(20) × 10^-3"],
  [1.23e-3, 0.02e-3, "1.230(20) × 10^-3"],
  [1.2e-5, 3.5e-5, "1.2(3.5) × 10^-5"],
  [1.23456, 0.01234, "1.235(12)"],
  [1.23456, 0.0999, "1.23(10)"],
  [0, 0.0039, "0.0(3.9) × 10^-3"],
  [0.001, 0.03, "0.1(3.0) × 10^-2"],
  [1.318e-120, 3.9e-123, "1.3180(39) × 10^-120"],
  [1.318e100, 3.9e97, "1.3180(39) × 10^100"],
  [2, 0, "2.000000(0)"],
  [0, 0, "0.000000(0)"],
])("estimate %s with uncertainty %s uses aligned parentheses notation", (value, error, text) => {
  const result = formatEstimateDisplay(value, error);
  expect(result.text).toBe(text);
  expect(result.latex).not.toContain("pm");
  if (value !== 0) expect(result.relative_percent).toBeCloseTo(Math.abs(error / value) * 100);
});

test.each([[12.3456, "12.3"], [0.00234, "0.002"], [-0.00234, "-0.002"],
  [0.001499, "0.001"], [0.09996, "0.100"], [1.23456, "1.23"], [-0.0001, "0.000"],
  [12345, "12300"], [null, "n/a"]])("comparison %s respects both precision caps", (value, expected) => {
  expect(formatComparisonDecimal(value)).toBe(expected);
});

test.each([[null, 1], [NaN, 1], [1, Infinity], [1, -0.1]])(
  "invalid estimate %s with error %s stays unavailable", (value, error) => {
    expect(formatEstimateDisplay(value, error).text).toBe("n/a");
  },
);

test("a provisional mean remains visible without an invented zero error", () => {
  const result = formatEstimateDisplay(1.5, null);
  expect(result.text).toBe("1.500000e+0");
  expect(result.relative_percent).toBeNull();
});


test.each([[12.3456, "12.3"], [1, "1.00"], [0, "0.00"], [-12.3456, "-12.3"],
  [0.0123456, "0.0123"], [12345, "12300"], [999.6, "1000"], [0.0009996, "0.00100"],
  [1.23456e-10, "0.000000000123"]])("formats %s with three significant digits and no exponent", (value, expected) => {
  expect(formatDecimalSignificant(value)).toBe(expected);
});

test("invalid decimal comparisons and exceptionally disparate uncertainty scales stay explicit", () => {
  expect(formatDecimalSignificant(null)).toBe("n/a");
  expect(formatDecimalSignificant(Infinity)).toBe("n/a");
  expect(formatEstimateDisplay(1e200, 1e-200).text).toContain("±");
  expect(formatEstimateDisplay(1e308, 1.79e308).text).not.toContain("Infinity");
});
