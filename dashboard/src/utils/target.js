const toFiniteNumber = (value) => {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
};

export const parseScalarTarget = (target) => {
  if (target == null) return null;

  const direct = toFiniteNumber(target);
  if (direct != null) {
    return { kind: "scalar", value: direct };
  }

  if (typeof target !== "object" || Array.isArray(target)) return null;

  const kind = typeof target.kind === "string" ? target.kind.toLowerCase() : null;
  const type = typeof target.type === "string" ? target.type.toLowerCase() : null;
  const value = toFiniteNumber(target.value);
  if (value == null) return null;

  if (kind == null && type == null) return { kind: "scalar", value };
  if (kind === "scalar" || kind === "value" || type === "scalar" || type === "value") {
    return { kind: "scalar", value };
  }

  return null;
};
