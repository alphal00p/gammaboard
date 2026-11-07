import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import RunInfo from "./RunInfo";
import { useRunPanels } from "../hooks/useRunPanels";

vi.mock("../hooks/useRunPanels", () => ({ useRunPanels: vi.fn(() => ({
  panelSpecs: ["run_identity", "checkpoint", "sampler_config", "evaluator_summary", "engine_config_error"].map((id) => ({ panel_id: id, label: id, kind: "text" })),
  panelStates: ["run_identity", "checkpoint", "sampler_config", "evaluator_summary", "engine_config_error"].map((id) => ({ panel_id: id, kind: "text", text: `${id} content` })),
})) }));

test("summary and effective settings partition the same response including configuration errors", () => {
  const { rerender } = render(<RunInfo runId={1} section="summary" />);
  expect(screen.getByText("run_identity content")).toBeVisible();
  expect(screen.getByText("checkpoint content")).toBeVisible();
  expect(screen.queryByText("sampler_config content")).not.toBeInTheDocument();
  rerender(<RunInfo runId={1} section="settings" />);
  expect(screen.queryByText("run_identity content")).not.toBeInTheDocument();
  expect(screen.getByText("sampler_config content")).toBeVisible();
  expect(screen.getByText("evaluator_summary content").compareDocumentPosition(screen.getByText("sampler_config content")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(screen.getByText("engine_config_error content")).toBeVisible();
});

test("summary combines run details, keeps task context next, and exposes recovery errors while live telemetry moves to tuning", () => {
  const states = [
    ...["run_identity", "run_lifecycle", "run_progress", "run_queue", "run_batch"].map((panel_id) => ({ panel_id, kind: "key_value", entries: [{ key: "value", label: panel_id, value: `${panel_id} data` }] })),
    { panel_id: "run_target", kind: "text", text: "real = 2" },
    { panel_id: "run_checkpoint", kind: "key_value", entries: [{ key: "state", label: "Status", value: "Failed" }, { key: "error", label: "Error", value: "Restore failed" }, { key: "saved_at", label: "Saved at", value: "Yesterday" }] },
  ];
  useRunPanels.mockReturnValue({ panelSpecs: states.map((state) => ({ panel_id: state.panel_id, kind: state.kind, label: state.panel_id })), panelStates: states });
  const { rerender } = render(<RunInfo runId={1}><div>Selected task summary</div></RunInfo>);
  expect(screen.getByText("Run overview").compareDocumentPosition(screen.getByText("Selected task summary")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  for (const id of ["run_identity", "run_lifecycle", "run_progress"]) expect(screen.getByText(`${id} data`)).toBeVisible();
  expect(screen.getByText("real = 2")).toBeVisible();
  expect(screen.queryByText("run_queue data")).not.toBeInTheDocument();
  expect(screen.queryByText("run_batch data")).not.toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("Restore failed");
  expect(screen.queryByText("Yesterday")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Checkpoint recovery" }));
  expect(screen.getByText("Yesterday")).toBeVisible();
  rerender(<RunInfo runId={1} section="tuning" />);
  expect(screen.getByText("run_queue data")).toBeVisible();
  expect(screen.getByText("run_batch data")).toBeVisible();
  expect(screen.queryByText("run_identity data")).not.toBeInTheDocument();
});
