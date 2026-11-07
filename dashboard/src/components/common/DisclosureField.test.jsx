import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import DisclosureField from "./DisclosureField";

test("the value and title share one toggle and expansion survives refreshed values", () => {
  const tree = (value) => <DisclosureField label="RSD" value={value}><p>Definition and precision</p><input aria-label="Detail input" /></DisclosureField>;
  const { rerender } = render(tree("1.23"));
  const toggle = screen.getByRole("button", { name: "RSD" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(within(toggle).getByText("1.23"));
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  fireEvent.change(screen.getByLabelText("Detail input"), { target: { value: "draft" } });
  rerender(tree("1.24"));
  expect(screen.getByLabelText("Detail input")).toHaveValue("draft");
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(within(toggle).getByText("RSD"));
  expect(screen.getByText("Definition and precision")).not.toBeVisible();
});

test("details and adjacent actions do not trigger surrounding row navigation", () => {
  const navigate = vi.fn(); const action = vi.fn();
  render(<div onClick={navigate}><DisclosureField label="Value" value="2.00" actions={<button onClick={action}>Apply</button>}>
    <span>Full value</span>
  </DisclosureField></div>);
  fireEvent.click(screen.getByRole("button", { name: "Value" }));
  fireEvent.click(screen.getByText("Full value"));
  expect(navigate).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Apply" }));
  expect(action).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: "Value" })).toHaveAttribute("aria-expanded", "true");
});

test("plain values retain alignment without pretending to have more information", () => {
  render(<DisclosureField label="Run ID" value="42" />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(screen.queryByText("▸")).not.toBeInTheDocument();
  expect(screen.getByText("42")).toBeVisible();
});
