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

  const { implementation, params } = splitKindConfig(evaluatorConfig, fallback);
  if (typeof params.observable_kind === "string") {
    return params.observable_kind;
  }

  if (implementation === "unit") return "scalar";
  if (implementation === "gammaloop") return "complex";
  if (implementation === "sinc_evaluator") return "complex";
  if (implementation === "sin_evaluator" || implementation === "symbolica") return "scalar";
  return fallback;
};
