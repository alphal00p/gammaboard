import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import NodeLaunchQueue from "./NodeLaunchQueue";

const request = (id, state) => ({ id, state, backend: "local", requested_count: 1,
  started_count: 1, args: { groups: [{ node_names: ["w-1"] }] },
  error: state === "failed" ? "worker exited before connecting" : null });

describe("node startup queue", () => {
  test("keeps outstanding work and failures visible, with successful attempts in collapsed history", () => {
    render(<NodeLaunchQueue requests={[
      request(1, "fulfilled"), request(2, "pending"), request(3, "starting"),
      request(4, "failed"), request(5, "canceled"),
    ]} />);
    const queue = within(screen.getByRole("table", { name: "node startup queue" }));
    expect(queue.getByText("pending")).toBeTruthy();
    expect(queue.getByText("starting")).toBeTruthy();
    expect(queue.getByText("worker exited before connecting")).toBeTruthy();
    expect(queue.queryByText("fulfilled")).toBeNull();
    expect(screen.queryByRole("table", { name: "node launch history" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Launch history (2)" }));
    const history = within(screen.getByRole("table", { name: "node launch history" }));
    expect(history.getByText("fulfilled")).toBeTruthy();
    expect(history.getAllByText("w-1")).toHaveLength(2);
  });

  test("a fulfilled launch leaves an empty queue even while the worker exists", () => {
    render(<NodeLaunchQueue requests={[request(1, "fulfilled")]} />);
    expect(screen.getByText("No outstanding launch requests.")).toBeTruthy();
    expect(screen.queryByRole("table", { name: "node startup queue" })).toBeNull();
  });
});
