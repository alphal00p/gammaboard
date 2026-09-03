import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { deleteTemplateFile, fetchTemplateFile, fetchTemplateList, saveTemplateFile } from "../services/api";
import { useTemplates } from "./useTemplates";

vi.mock("../services/api", () => ({
  deleteTemplateFile: vi.fn(),
  fetchTemplateFile: vi.fn(),
  fetchTemplateList: vi.fn(),
  saveTemplateFile: vi.fn(),
}));

describe("useTemplates", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  test("loads one template kind", async () => {
    fetchTemplateList.mockResolvedValue(["first.toml"]);
    const { result } = renderHook(() => useTemplates({ kind: "runs" }));

    await waitFor(() => expect(result.current.templates).toEqual(["first.toml"]));

    expect(fetchTemplateList).toHaveBeenCalledWith("runs", expect.any(AbortSignal));
  });

  test("does not load while disabled", () => {
    renderHook(() => useTemplates({ kind: "nodes", enabled: false }));
    expect(fetchTemplateList).not.toHaveBeenCalled();
  });

  test("loads, saves, and deletes files while reconciling the list locally", async () => {
    fetchTemplateList.mockResolvedValue(["first.toml"]);
    fetchTemplateFile.mockResolvedValue({ name: "first.toml", toml: "name = 'first'" });
    saveTemplateFile.mockResolvedValue({ name: "second.toml", toml: "name = 'second'" });
    deleteTemplateFile.mockResolvedValue({ deleted: true, name: "first.toml" });
    const { result } = renderHook(() => useTemplates({ kind: "tasks" }));
    await waitFor(() => expect(result.current.templates).toEqual(["first.toml"]));

    await expect(result.current.load("first.toml")).resolves.toBe("name = 'first'");
    await act(async () => result.current.save("second", "name = 'second'"));
    expect(result.current.templates).toEqual(["first.toml", "second.toml"]);
    await act(async () => result.current.remove("first.toml"));
    expect(result.current.templates).toEqual(["second.toml"]);

    expect(fetchTemplateFile).toHaveBeenCalledWith("tasks", "first.toml", undefined);
    expect(saveTemplateFile).toHaveBeenCalledWith("tasks", { name: "second", toml: "name = 'second'" }, undefined);
    expect(deleteTemplateFile).toHaveBeenCalledWith("tasks", "first.toml", undefined);
    expect(fetchTemplateList).toHaveBeenCalledTimes(1);
  });
});
