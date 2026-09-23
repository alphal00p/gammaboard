import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import QueueTuningPanel from "./QueueTuningPanel";

const defaults = {
  queue_buffer: 1, target_batch_eval_ms: 2000, batch_size_deadband_ratio: 0.15,
  batch_size_cooldown_ticks: 3, max_batch_size: 100000, max_queue_size: 200,
  max_batches_per_tick: 100, max_insert_bundle_size: 5, max_concurrent_insert_tasks: 8,
  completed_batch_fetch_limit: 100,
};

describe("queue bulk generation toggle", () => {
  test("defaults old runs to false and submits a boolean task override", async () => {
    const onSave = vi.fn();
    render(<QueueTuningPanel run={{ queue_tuning_defaults: defaults }} runId={1}
      task={{ id: 2, is_sample: true }} authenticated onSave={onSave} />);
    const toggle = screen.getByRole("switch", { name: "Bulk Sample Generation" });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Apply" })); });
    expect(onSave).toHaveBeenCalledWith({ ...defaults, bulk_sample_generation: true });
  });

  test("a false task override takes precedence over a true run default", async () => {
    const onSave = vi.fn();
    render(<QueueTuningPanel run={{ queue_tuning_defaults: { ...defaults, bulk_sample_generation: true } }} runId={1}
      task={{ id: 2, is_sample: true, queue_tuning: { bulk_sample_generation: false } }} authenticated onSave={onSave} />);
    expect(screen.getByRole("switch", { name: "Bulk Sample Generation" })).not.toBeChecked();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Apply" })); });
    expect(onSave).toHaveBeenCalledWith({ ...defaults, bulk_sample_generation: false });
  });
});
