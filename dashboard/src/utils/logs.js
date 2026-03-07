export const buildLogSearchText = (entry) => {
  const message = entry?.message || "";
  let fields = "";
  try {
    fields = JSON.stringify(entry?.fields || {});
  } catch {
    fields = "";
  }
  return `${message} ${fields}`.toLowerCase();
};

export const mergeLogsAsc = (previous, incoming, maxSize) => {
  if (!Array.isArray(incoming) || incoming.length === 0) return previous;
  const out = Array.isArray(previous) ? [...previous] : [];
  const seen = new Set(out.map((entry) => entry?.id));
  let hasNew = false;
  for (const entry of incoming) {
    if (!entry || entry.id == null) continue;
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    out.push(entry);
    hasNew = true;
  }
  if (!hasNew) return previous;
  return out.length > maxSize ? out.slice(out.length - maxSize) : out;
};
