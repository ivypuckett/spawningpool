import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listEntities, showEntity } from "./api";

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

describe("showEntity", () => {
  beforeEach(() => vi.resetAllMocks());

  it("invokes show_entity with kind and name and returns the result", async () => {
    const definition = { name: "anthropic", api: "anthropic-messages", base_url: "https://api.anthropic.com" };
    vi.mocked(invoke).mockResolvedValue(definition);

    const result = await showEntity("provider", "anthropic");

    expect(invoke).toHaveBeenCalledWith("show_entity", { kind: "provider", name: "anthropic" });
    expect(result).toEqual(definition);
  });
});
