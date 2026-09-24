import { expect, test } from "vitest";
import { formatEstimateDisplay } from "./formatters";

test.each([
  [-1.318e-4, 0.0039e-4, "-1.3180(39) × 10^-4"],
  [1.23456, 0.01234, "1.235(12) × 10^0"],
  [1.23456, 0.0999, "1.23(10) × 10^0"],
  [0, 0.0039, "0.0(39) × 10^-3"],
  [0.001, 0.03, "0.1(30) × 10^-2"],
  [1.318e-120, 3.9e-123, "1.3180(39) × 10^-120"],
  [1.318e100, 3.9e97, "1.3180(39) × 10^100"],
  [2, 0, "2.000000(0) × 10^0"],
  [0, 0, "0.000000(0) × 10^0"],
])("estimate %s with uncertainty %s uses aligned parentheses notation", (value, error, text) => {
  const result = formatEstimateDisplay(value, error);
  expect(result.text).toBe(text);
  expect(result.latex).not.toContain("pm");
  if (value !== 0) expect(result.relative_percent).toBeCloseTo(Math.abs(error / value) * 100);
});

test.each([[null, 1], [1, null], [NaN, 1], [1, Infinity], [1, -0.1]])(
  "invalid estimate %s with error %s stays unavailable", (value, error) => {
    expect(formatEstimateDisplay(value, error).text).toBe("n/a");
  },
);
