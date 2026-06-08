import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("./api", () => ({ listEntities: vi.fn() }));
import { listEntities } from "./api";
import { registry, selection, loadRegistry } from "./stores";

describe("registry store", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    registry.set(null);
    selection.set(null);
  });

  it("loadRegistry fills the registry store from the api", async () => {
    const snapshot = { providers: ["anthropic"], models: [], specialists: [], tools: [], registry_path: "/tmp/registry.json" };
    vi.mocked(listEntities).mockResolvedValue(snapshot);

    await loadRegistry();

    expect(get(registry)).toEqual(snapshot);
  });

  it("selection starts null and can be set", () => {
    expect(get(selection)).toBeNull();
    selection.set({ kind: "provider", name: "anthropic" });
    expect(get(selection)).toEqual({ kind: "provider", name: "anthropic" });
  });
});
