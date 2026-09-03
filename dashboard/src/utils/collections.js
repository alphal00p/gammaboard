export const asArray = (value) => (Array.isArray(value) ? value : []);

export const isPlainObject = (value) => value != null && typeof value === "object" && !Array.isArray(value);
