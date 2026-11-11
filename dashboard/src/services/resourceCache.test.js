import { afterEach, expect, test, vi } from "vitest";
import { clearResourceCache, readResourceCache, writeResourceCache } from "./resourceCache";
afterEach(() => { clearResourceCache(); vi.useRealTimers(); });
test("session cache evicts least-used entries and expires old snapshots", () => {
  vi.useFakeTimers();
  for (let i=0;i<64;i++) writeResourceCache(String(i), { result:i });
  expect(readResourceCache("0")).toEqual({ result:0 });
  writeResourceCache("new", { result:64 });
  expect(readResourceCache("1")).toBeNull();
  expect(readResourceCache("0")).toEqual({ result:0 });
  vi.advanceTimersByTime(30*60*1000+1);
  expect(readResourceCache("0")).toBeNull();
});
