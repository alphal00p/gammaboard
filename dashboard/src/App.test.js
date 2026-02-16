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

  test("renders integration mean heading", () => {
    render(<App />);
    const chartHeading = screen.getByText(/Integration Mean/i);
    expect(chartHeading).toBeInTheDocument();
  });

  test("shows empty state message when no run is selected", () => {
    render(<App />);
    const emptyMessage = screen.getByText(/Select a run to view data/i);
    expect(emptyMessage).toBeInTheDocument();
  });
});
