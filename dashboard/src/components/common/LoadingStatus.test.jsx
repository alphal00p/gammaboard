import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import LoadingStatus from "./LoadingStatus";

afterEach(() => vi.useRealTimers());

test("fast polls do not flash a spinner; slow requests expose accessible status", () => {
  vi.useFakeTimers();
  const { rerender } = render(<LoadingStatus loading label="Loading observables…" />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() => vi.advanceTimersByTime(100));
  rerender(<LoadingStatus loading={false} />);
  act(() => vi.advanceTimersByTime(100));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  rerender(<LoadingStatus loading label="Updating observables…" />);
  act(() => vi.advanceTimersByTime(150));
  expect(screen.getByRole("status")).toHaveTextContent("Updating observables…");
  rerender(<LoadingStatus loading={false} />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
