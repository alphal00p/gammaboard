export const getTaskKindLabel = (task) => task?.task?.kind ?? "unknown";

export const asTaskList = (tasks) => (Array.isArray(tasks) ? tasks : []);

const getGeometryPointCount = (taskSpec) => {
  if (!taskSpec || typeof taskSpec !== "object") return null;
  if (taskSpec.kind === "image") {
    const uCount = Number(taskSpec.geometry?.u_linspace?.count);
    const vCount = Number(taskSpec.geometry?.v_linspace?.count);
    if (Number.isFinite(uCount) && Number.isFinite(vCount)) {
      return uCount * vCount;
    }
  }
  if (taskSpec.kind === "plot_line") {
    const count = Number(taskSpec.geometry?.linspace?.count);
    if (Number.isFinite(count)) {
      return count;
    }
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

export const getCurrentSampleTask = (tasks) => {
  const currentTask = getCurrentTask(tasks);
  return currentTask?.task?.kind === "sample" ? currentTask : null;
};
