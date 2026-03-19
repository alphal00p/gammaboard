import { isPlainObject } from "./collections";

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

export const deriveObservableImplementation = (evaluatorConfig, observablePayload, fallback = "unknown") => {
  const payload = toConfigObject(observablePayload);
  if (typeof payload.kind === "string") {
    return payload.kind;
  }

  const { params } = splitKindConfig(evaluatorConfig, fallback);
  if (typeof params.observable_kind === "string") {
    return params.observable_kind;
  }

  return fallback;
};
