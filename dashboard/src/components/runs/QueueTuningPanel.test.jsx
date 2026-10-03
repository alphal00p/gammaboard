import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import QueueTuningPanel from "./QueueTuningPanel";

const defaults = { target_batch_eval_ms: 2000, max_batch_size: 100000, max_generation_size: 262144 };
const props = { run: { queue_tuning_defaults: defaults }, runId: 1,
  task: { id: 2, is_sample: true }, authenticated: true };

describe("queue tuning", () => {
  test("preserves an advanced fixed batch override when changing the target", async () => {
    const onSave = vi.fn();
    render(<QueueTuningPanel {...props} task={{ ...props.task, queue_tuning: { fixed_batch_size: 4096 } }} onSave={onSave} />);
    fireEvent.change(screen.getByLabelText("Target Evaluation Time (ms)"), { target: { value: "500" } });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Apply" })); });
    expect(onSave).toHaveBeenCalledWith({ ...defaults, target_batch_eval_ms: 500, fixed_batch_size: 4096 });
  });
  test("omits obsolete controls from old stored defaults", async () => {
    const onSave = vi.fn();
    render(<QueueTuningPanel {...props} run={{ queue_tuning_defaults: { ...defaults, queue_buffer: 8, bulk_sample_generation: false } }} onSave={onSave} />);
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Apply" })); });
    expect(onSave).toHaveBeenCalledWith(defaults);
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
  });
  test("rejects a nonpositive evaluation target", async () => {
    const onSave = vi.fn();
    render(<QueueTuningPanel {...props} onSave={onSave} />);
    fireEvent.change(screen.getByLabelText("Target Evaluation Time (ms)"), { target: { value: "0" } });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Apply" })); });
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Invalid value");
  });
  test("controller-owned children cannot be tuned independently", () => {
    render(<QueueTuningPanel {...props} run={{ parent_run_id: 9 }} />);
    expect(screen.getByText(/managed by its parent/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
  });
});
