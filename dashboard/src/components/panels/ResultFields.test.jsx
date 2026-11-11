import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import { KeyValuePanel } from "./BasicPanels";

const estimate = (value) => ({ kind: "estimate", value, error: 0.0025, estimator: "stratified", sample_count: 128 });
const state = { panel_id: "estimate_summary", entries: [
  { key: "component_real_mean", label: "Real mean", value: estimate(5.4123) },
  { key: "component_imag_mean", label: "Imag mean", value: estimate(1.2345) },
  { key: "component_real_mean_abs", label: "|Real| mean", value: estimate(6.5432) },
  { key: "component_imag_mean_abs", label: "|Imag| mean", value: estimate(3.4567) },
  { key: "target_comparison_real", label: "Real vs target", value: { kind: "target_comparison", value: 5.4123, error: 0.0025, target: 5.4, delta_percent: 12.3456, delta_sigma: 1.23456 } },
  { key: "component_real_rsd", label: "Real RSD", value: 0.05 },
  { key: "component_imag_rsd", label: "Imag RSD", value: 0.01 },
  { key: "ess", label: "Normalized training ESS", value: 0.8 },
  { key: "diagnostics_basis", label: "Diagnostics", value: "old combined definition" },
] };

test("components keep mean, target and RSD together with field-specific definitions", () => {
  const { container } = render(<KeyValuePanel title="Result" state={state} />);
  const real = container.querySelector('[data-result-component="real"]');
  expect(within(real).getAllByRole("button").map((button) => button.getAttribute("aria-label"))).toEqual(["Real mean", "Real vs target", "Real RSD", "|Real| mean"]);
  expect(container.querySelectorAll('[data-result-component]')).toHaveLength(2);
  expect(within(container.querySelector('[data-result-component="imag"]')).getByRole("button", { name: "|Imag| mean" })).toHaveTextContent("3.4567(25)");
  expect(within(container.querySelector('[data-result-component="imag"]')).getByText("not set")).toBeVisible();
  const mean = screen.getByRole("button", { name: "Real mean" });
  expect(mean).toHaveTextContent("5.4123(25)");
  const target = screen.getByRole("button", { name: "Real vs target" });
  expect(target).toHaveTextContent("12.3%");
  expect(target).toHaveTextContent("1.23σ");
  expect(target).not.toHaveTextContent("t=");
  fireEvent.click(screen.getByRole("button", { name: "Real RSD" }));
  expect(screen.getByText(/RSD = sqrt/)).toBeVisible();
  expect(screen.queryByText(/Normalized training ESS =/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Normalized training ESS" }));
  expect(screen.getByText(/Normalized training ESS =/)).toBeVisible();
  expect(screen.queryByText("old combined definition")).not.toBeInTheDocument();
});

test("a signed component named abs keeps its own mean and absolute mean", () => {
  const { container } = render(<KeyValuePanel title="Result" state={{ panel_id: "estimate_summary", entries: [
    { key: "component_abs_mean", label: "abs Mean", value: estimate(-1) },
    { key: "component_abs_mean_abs", label: "|abs| Mean", value: estimate(2) },
  ] }} />);
  const component = container.querySelector('[data-result-component="abs"]');
  expect(within(component).getByRole("button", { name: "abs Mean", exact: true })).toBeVisible();
  expect(within(component).getByRole("button", { name: "|abs| Mean", exact: true })).toBeVisible();
});
