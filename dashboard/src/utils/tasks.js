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
  if (task?.task?.kind === "set_accumulator") {
    return "-";
  }
  const raw =
    Number(task?.task?.stop_condition?.max_samples) || getGeometryPointCount(task?.task);
  return Number.isFinite(raw) ? raw.toLocaleString() : "unbounded";
};

const taskQueueOrder = (task, index) => {
  const sequence = Number(task?.sequence_nr);
  return Number.isFinite(sequence) ? sequence : index;
};

const lastTaskByQueueOrder = (tasks, predicate) => {
  return asTaskList(tasks)
    .map((task, index) => ({ task, order: taskQueueOrder(task, index) }))
    .filter(({ task }) => predicate(task))
    .sort((left, right) => right.order - left.order)
    .at(0)?.task ?? null;
};

export const getCurrentTask = (tasks) => {
  const taskList = asTaskList(tasks);
  return (
    taskList.find((task) => task.state === "active") ||
    lastTaskByQueueOrder(taskList, (task) => task.state === "completed") ||
    taskList.find((task) => task.state === "pending") ||
    lastTaskByQueueOrder(taskList, (task) => task.state === "failed")
  );
};
