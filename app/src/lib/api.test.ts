import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listEntities } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("listEntities", () => {
  beforeEach(() => vi.resetAllMocks());

  it("invokes list_entities and returns the snapshot", async () => {
    const snapshot = {
      providers: ["anthropic"],
      models: [],
      specialists: [],
      tools: [],
      registry_path: "/tmp/registry.json",
    };
    vi.mocked(invoke).mockResolvedValue(snapshot);

    const result = await listEntities();

    expect(invoke).toHaveBeenCalledWith("list_entities");
    expect(result).toEqual(snapshot);
  });
});
