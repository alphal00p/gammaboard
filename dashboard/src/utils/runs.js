export const formatRunLabel = (run) => {
  if (!run) return "Unknown run";
  const label = run.run_name ? run.run_name : "Unnamed run";
  return run.parent_run_id != null ? `- ${label}` : label;
};

const deriveRunLifecycle = (run) =>
  typeof run?.lifecycle_state === "string" && run.lifecycle_state.trim() ? run.lifecycle_state : "unknown";

export const formatRunSecondaryLabel = (run) =>
  [
    run?.parent_run_id != null ? `child of #${run.parent_run_id}` : null,
    deriveRunLifecycle(run),
    `completed samples ${Number(run?.nr_completed_samples_including_children ?? 0).toLocaleString()}`,
  ]
    .filter(Boolean)
    .join(" | ");

export const orderRunsForSelector = (runs) => {
  const items = Array.isArray(runs) ? runs : [];
  const childrenByParent = new Map();
  const roots = [];

  for (const run of items) {
    if (run?.parent_run_id == null) {
      roots.push(run);
      continue;
    }
    const key = Number(run.parent_run_id);
    const children = childrenByParent.get(key) || [];
    children.push(run);
    childrenByParent.set(key, children);
  }

  const sortChildren = (left, right) => {
    const leftLabel = Number(left?.spawn_label);
    const rightLabel = Number(right?.spawn_label);
    if (Number.isFinite(leftLabel) && Number.isFinite(rightLabel) && leftLabel !== rightLabel) {
      return leftLabel - rightLabel;
    }
    return Number(left?.run_id || 0) - Number(right?.run_id || 0);
  };

  const ordered = [];
  for (const run of roots) {
    ordered.push(run);
    ordered.push(...(childrenByParent.get(Number(run.run_id)) || []).sort(sortChildren));
  }

  for (const run of items) {
    if (run?.parent_run_id == null) continue;
    if (!ordered.some((candidate) => candidate?.run_id === run.run_id)) ordered.push(run);
  }

  return ordered;
};
