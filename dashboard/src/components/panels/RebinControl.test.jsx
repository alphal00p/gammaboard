import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import RebinControl from "./RebinControl";

test("large step controls change the merge factor, respect bounds and recover from an unfinished draft", () => {
  function Harness() {
    const [value, setValue] = useState(1);
    return <RebinControl value={value} max={3} onChange={setValue} />;
  }
  render(<Harness />);
  const input = screen.getByRole("spinbutton", { name: "Merge adjacent bins" });
  const up = screen.getByRole("button", { name: "Increase merged bins" });
  const down = screen.getByRole("button", { name: "Decrease merged bins" });
  expect(down).toBeDisabled();
  fireEvent.click(up);
  expect(input).toHaveValue(2);
  fireEvent.click(up);
  expect(input).toHaveValue(3);
  expect(up).toBeDisabled();
  fireEvent.change(input, { target: { value: "" } });
  fireEvent.click(down);
  expect(input).toHaveValue(2);
  fireEvent.change(input, { target: { value: "1" } });
  expect(down).toBeDisabled();
});
