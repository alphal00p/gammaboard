export function workerAssignmentReason(run) {
  if (!run) return "Select a worker pool.";
  if (run.parent_run_id != null) return "Assign workers to the parent run's pool.";
  if (run.lifecycle_state === "completed") return "Append new tasks before assigning workers to this run.";
  if (run.lifecycle_state === "failed") return "Recover the failed task before assigning workers to this run.";
  return "";
}

// Current work may still be draining after desired assignments were cleared.
export const hasWorkerAssignment = (worker) => [
  worker?.pool_run_id, worker?.current_run_id, worker?.desired_run_id,
  worker?.pool_role, worker?.current_role, worker?.desired_role,
].some((value) => value != null);
