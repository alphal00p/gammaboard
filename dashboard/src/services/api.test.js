import { afterEach, expect, test, vi } from "vitest";
import { fetchSession } from "./api";

afterEach(() => vi.unstubAllGlobals());

test("session startup uses POST so same-origin browsers send their Origin", async () => {
  const fetch = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({ authenticated: true }),
  });
  vi.stubGlobal("fetch", fetch);
  expect(await fetchSession()).toEqual({ authenticated: true });
  expect(fetch).toHaveBeenCalledWith("/api/auth/session", expect.objectContaining({
    method: "POST",
    credentials: "include",
  }));
});
