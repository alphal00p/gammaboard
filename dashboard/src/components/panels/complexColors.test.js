import { expect, test } from "vitest";
import { complexPhaseColorForValue } from "./complexColors";

test("the phase key and raster share the complex-plane hue convention and magnitude saturation", () => {
  expect(complexPhaseColorForValue(1, 0, 1)).toEqual([0, 255, 255]);
  expect(complexPhaseColorForValue(0, 1, 1)).toEqual([128, 0, 255]);
  expect(complexPhaseColorForValue(-1, 0, 1)).toEqual([255, 0, 0]);
  expect(complexPhaseColorForValue(0, -1, 1)).toEqual([128, 255, 0]);
  expect(complexPhaseColorForValue(0, 0, 1)).toEqual([255, 255, 255]);
  expect(complexPhaseColorForValue(1, 0, 2)).toEqual([128, 255, 255]);
});
