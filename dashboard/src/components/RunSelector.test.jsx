import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import RunSelector from "./RunSelector";

test("child options keep their dash and add indentation", () => {
  render(<RunSelector runs={[{run_id:1,run_name:"Campaign"},{run_id:2,run_name:"Child",parent_run_id:1}]} selectedRun={1} />);
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Select run" }));
  const parent = screen.getByRole("option", { name: /^Campaign/ });
  const child = screen.getByRole("option", { name: /^- Child/ });
  expect(parseFloat(getComputedStyle(child).paddingLeft)).toBeGreaterThan(parseFloat(getComputedStyle(parent).paddingLeft));
});
