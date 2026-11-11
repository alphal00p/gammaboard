import "@testing-library/jest-dom";
import { afterEach } from "vitest";
import { clearResourceCache } from "./services/resourceCache";
afterEach(clearResourceCache);
import { TextDecoder, TextEncoder } from "util";

// jsdom has no layout/media queries; these APIs are used by MUI and CodeMirror.
if (!window.matchMedia) {
  window.matchMedia = (media) => ({ matches: false, media, addListener() {}, removeListener() {},
    addEventListener() {}, removeEventListener() {} });
}
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [];
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

if (typeof global.TextEncoder === "undefined") {
  global.TextEncoder = TextEncoder;
}

if (typeof global.TextDecoder === "undefined") {
  global.TextDecoder = TextDecoder;
}
