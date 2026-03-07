const isPlainObject = (value) => value && typeof value === "object" && !Array.isArray(value);

export const toConfigObject = (value) => {
  if (!value) return {};
  if (isPlainObject(value)) return value;
  if (typeof value !== "string") return {};

  try {
    const parsed = JSON.parse(value);
    return isPlainObject(parsed) ? parsed : {};
  } catch {
    return {};
  }
};

export const splitKindConfig = (config, fallbackImplementation = "unknown", fallbackParams = {}) => {
  const parsedConfig = toConfigObject(config);
  if (isPlainObject(parsedConfig) && typeof parsedConfig.kind === "string") {
    const { kind, ...params } = parsedConfig;
    return { implementation: kind, params };
  }

  return {
    implementation: fallbackImplementation,
    params: toConfigObject(fallbackParams),
  };
};
