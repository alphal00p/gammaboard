import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import ConfigurationTree from "./ConfigurationTree";

const entries = (scale = 0.25) => [{ key: "evaluator", label: "Evaluator", value: {
  kind: "symbolica", args: ["x", { scale, active: false }], source: { "mass.top": 172.5 }, empty: [], missing: null,
} }];

test("nested arrays and objects expand independently and remain expanded across polling", () => {
  const { rerender, container } = render(<ConfigurationTree entries={entries()} />);
  expect(screen.getByRole("button", { name: "Evaluator" })).toHaveAttribute("aria-expanded", "true");
  const args = screen.getByRole("button", { name: "args" });
  expect(args).toHaveTextContent("2 items");
  fireEvent.click(args);
  fireEvent.click(screen.getByRole("button", { name: "[1]" }));
  expect(screen.getByText("0.25")).toBeVisible();
  const scale = container.querySelector('[data-config-path=\'["evaluator","args","1","scale"]\']');
  expect(getComputedStyle(scale.querySelector('button > span')).paddingLeft).toBe("36px");
  rerender(<ConfigurationTree entries={entries(0.5)} />);
  expect(screen.getByText("0.5")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "source" }));
  expect(screen.getByRole("button", { name: "mass.top" })).toBeVisible();
  expect(screen.getByText("none")).toBeVisible();
  expect(screen.getByText("0 items")).toBeVisible();
  fireEvent.click(args);
  expect(screen.queryByRole("button", { name: "[1]" })).not.toBeInTheDocument();
  fireEvent.click(args);
  expect(screen.getByText("0.5")).toBeVisible();
});

test("earlier dotted configuration keys retain their hierarchy", () => {
  render(<ConfigurationTree entries={[
    { key: "runner.queue.max_batch", label: "Runner Queue Max Batch", value: 64 },
    { key: "runner.queue.timeout", label: "Runner Queue Timeout", value: 2 },
  ]} />);
  fireEvent.click(screen.getByRole("button", { name: "queue" }));
  expect(screen.getByText("64")).toBeVisible();
  expect(screen.getByText("max_batch")).toBeVisible();
});
