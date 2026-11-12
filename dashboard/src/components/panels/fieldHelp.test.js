import { expect, test } from "vitest";
import { fieldHelp } from "./fieldHelp";

test("campaign scheduling and stratum allocation have distinct definitions", () => {
  expect(fieldHelp("allocation_window_samples", "Allocation Window", "task_summary")).toContain("scheduling window");
  expect(fieldHelp("allocation_window_samples", "Allocation Window", "task_summary")).not.toContain("this stratum");
  expect(fieldHelp("", "Allocation", "stratum_results")).toContain("this stratum");
  expect(fieldHelp("", "Observed allocation", "strata")).toContain("this stratum");
  expect(fieldHelp("allocated_core_hours", "Allocated core-hours", "performance_overview")).toBeNull();
  expect(fieldHelp("", "Matches", "unrelated")).toBeNull();
});

test("campaign sample scopes and weighted variance share are explained at their fields", () => {
  expect(fieldHelp("min_samples_per_child", "Pilot Minimum / Sub-run", "task_summary")).toContain("final publishing stage");
  expect(fieldHelp("max_total_samples", "Sample Cap", "task_summary")).toContain("including training");
  expect(fieldHelp("", "var ctrb (%)", "campaign_children")).toContain("coefficient-squared");
  expect(fieldHelp("", "samples", "campaign_children")).toContain("published result");
  expect(fieldHelp("", "Share of reported variance (real)", "strata")).toContain("stratum");
  expect(fieldHelp("", "Exact requested p")).toContain("proposal");
});
