export const getTaskKindLabel = (task) => task?.task?.kind ?? "unknown";

export const getTaskTargetLabel = (task) => {
  const raw = Number(task?.task?.nr_samples);
  return Number.isFinite(raw) ? raw.toLocaleString() : "unbounded";
};

export const getCurrentTask = (tasks) =>
  (Array.isArray(tasks) ? tasks : []).find((task) => task.state === "active") ||
  (Array.isArray(tasks) ? tasks : []).find((task) => task.state === "pending") ||
  (Array.isArray(tasks) ? tasks : []).find((task) => task.state === "completed") ||
  null;

export const getCurrentSampleTask = (tasks) => {
  const currentTask = getCurrentTask(tasks);
  return currentTask?.task?.kind === "sample" ? currentTask : null;
};
