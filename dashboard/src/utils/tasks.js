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

export const formatTaskSnapshotRef = (snapshotIdValue) => {
  if (snapshotIdValue == null) {
    return "inherit";
  }
  const snapshotId = Number(snapshotIdValue);
  if (Number.isFinite(snapshotId)) {
    return String(snapshotId);
  }
  return "inherit";
};

export const formatTaskSpawnOrigin = (task) => {
  if (task?.spawned_from_snapshot_id == null) {
    return "auto";
  }
  const snapshotId = Number(task?.spawned_from_snapshot_id);
  if (Number.isFinite(snapshotId)) {
    return String(snapshotId);
  }
  return "auto";
};
