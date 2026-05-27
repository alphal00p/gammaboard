import { describe, expect, test } from "vitest";
import { getCurrentTask } from "./tasks";

const task = (id, state, sequence_nr = Number(id)) => ({
  id: String(id),
  state,
  sequence_nr,
});

describe("getCurrentTask", () => {
  test("prefers the active task", () => {
    expect(getCurrentTask([task(1, "completed"), task(2, "active"), task(3, "completed")])?.id).toBe("2");
  });

  test("uses the last completed task before pending work", () => {
    expect(getCurrentTask([task(1, "completed"), task(2, "completed"), task(3, "pending")])?.id).toBe("2");
  });

  test("uses the first pending task when nothing has completed yet", () => {
    expect(getCurrentTask([task(1, "pending"), task(2, "pending")])?.id).toBe("1");
  });

  test("falls back to the last failed task for finished failing queues", () => {
    expect(getCurrentTask([task(1, "failed"), task(2, "failed")])?.id).toBe("2");
  });
});
