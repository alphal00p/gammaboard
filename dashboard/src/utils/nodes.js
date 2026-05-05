export const nodeNameOf = (node) => node?.node_name ?? null;

export const compareNodeNames = (left, right) =>
  String(left || "").localeCompare(String(right || ""), undefined, {
    numeric: true,
    sensitivity: "base",
  });

export const compareNodesByName = (left, right) => compareNodeNames(nodeNameOf(left), nodeNameOf(right));
