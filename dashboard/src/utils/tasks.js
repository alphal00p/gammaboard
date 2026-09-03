import { asArray } from "./collections";

export const asTaskList = (tasks) => asArray(tasks);

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
