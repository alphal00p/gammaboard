import { asArray } from "./collections";

export const getTaskKindLabel = (task) => task?.task?.kind ?? "unknown";

export const asTaskList = (tasks) => asArray(tasks);

const getGeometryPointCount = (taskSpec) => {
  if (!taskSpec || typeof taskSpec !== "object") return null;
  const uCount = Number(taskSpec.geometry?.u_linspace?.count);
  const vCount = Number(taskSpec.geometry?.v_linspace?.count);
  if (Number.isFinite(uCount) && Number.isFinite(vCount)) {
    return uCount * vCount;
  }
  const count = Number(taskSpec.geometry?.linspace?.count);
  if (Number.isFinite(count)) {
    return count;
  }
  return null;
};

export const getTaskTargetLabel = (task) => {
  const raw = Number(task?.task?.nr_samples) || getGeometryPointCount(task?.task);
  return Number.isFinite(raw) ? raw.toLocaleString() : "unbounded";
};

export const getCurrentTask = (tasks) =>
  asTaskList(tasks).find((task) => task.state === "active") ||
  asTaskList(tasks).find((task) => task.state === "pending") ||
  asTaskList(tasks).find((task) => task.state === "completed") ||
  null;

export const formatTaskSnapshotRef = (snapshot) => {
  if (!snapshot || typeof snapshot !== "object") return "inherit";
  const runId = Number(snapshot.run_id);
  const taskId = Number(snapshot.task_id);
  if (Number.isFinite(runId) && Number.isFinite(taskId)) {
    return `${runId}:${taskId}`;
  }
  return "inherit";
};

export const formatTaskSpawnOrigin = (task) => {
  const runId = Number(task?.spawned_from_run_id);
  const taskId = Number(task?.spawned_from_task_id);
  if (Number.isFinite(runId) && Number.isFinite(taskId)) {
    return `${runId}:${taskId}`;
  }
  return "auto";
};
