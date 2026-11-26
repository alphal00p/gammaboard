// Idempotency keys are opaque strings to the server. getRandomValues also
// works on plain HTTP dashboards, where crypto.randomUUID is unavailable.
export function createRequestId() {
  if (typeof globalThis.crypto?.getRandomValues !== "function") {
    throw new Error("This browser cannot generate a request ID.");
  }
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
