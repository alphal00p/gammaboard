import { render, screen } from "@testing-library/react";
import App from "./App";

/**
 * Basic smoke test for the App component
 *
 * Tests that the main application renders without crashing
 * and contains expected core elements.
 */
describe("App Component", () => {
  test("renders Gammaboard title", () => {
    render(<App />);
    const titleElement = screen.getByText(/Gammaboard/i);
    expect(titleElement).toBeInTheDocument();
  });

  test("renders connection status component", () => {
    render(<App />);
    // ConnectionStatus should show "Disconnected" initially
    const statusElement = screen.getByText(/Disconnected/i);
    expect(statusElement).toBeInTheDocument();
  });

  test("renders mode tabs", () => {
    render(<App />);
    expect(screen.getByRole("tab", { name: /Runs/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Workers/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Logs/i })).toBeInTheDocument();
  });

  test("shows no-runs empty state when run list is empty", () => {
    render(<App />);
    const emptyMessage = screen.getByText(/No runs available/i);
    expect(emptyMessage).toBeInTheDocument();
  });
});
