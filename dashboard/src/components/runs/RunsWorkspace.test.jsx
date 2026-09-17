import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import RunsWorkspace from "./RunsWorkspace";

vi.mock("../../auth/AuthProvider", () => ({ useAuth: () => ({ authenticated: true }) }));
vi.mock("../../hooks/useRunTasks", () => ({ useRunTasks: () => ({ tasks: [] }) }));
vi.mock("../common/RunScopedWorkspace", () => ({ default: ({ children }) => children }));
vi.mock("../RunInfo", () => ({ default: () => null }));
vi.mock("../TaskQueuePanel", () => ({ default: () => null }));
vi.mock("../TaskOutputPanel", () => ({ default: () => null }));
vi.mock("./CloneRunDialog", () => ({ default: () => null }));
vi.mock("./TomlActionDialog", () => ({ default: () => null }));

const runs = [
  { run_id: 1, run_name: "parent", kind: "integration_campaign" },
  { run_id: 2, run_name: "child", parent_run_id: 1, kind: "integration" },
];

describe("worker pool controls", () => {
  test("child pages explain ownership and navigate to the parent without mutation controls", async () => {
    const onSelectRun = vi.fn();
    render(<RunsWorkspace runs={runs} selectedRun={2} onSelectRun={onSelectRun} />);
    expect(await screen.findByText(/Workers are allocated by the parent/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Assign / Resume" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove evaluators" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pause Run" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Manage parent workers" }));
    expect(onSelectRun).toHaveBeenCalledWith(1);
  });

  test("parent pages expose pool controls", async () => {
    render(<RunsWorkspace runs={runs} selectedRun={1} />);
    expect(await screen.findByRole("button", { name: "Assign / Resume" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove evaluators" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pause Run" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Manage parent workers" })).not.toBeInTheDocument();
  });
});
