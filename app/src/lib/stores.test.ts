import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("./api", () => ({ listEntities: vi.fn() }));

import { listen } from "@tauri-apps/api/event";
import { listEntities } from "./api";
import { registry, selection, loadRegistry, watchRegistry } from "./stores";

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

  it("loadRegistry clears selection when the selected entity is absent from the new snapshot", async () => {
    selection.set({ kind: "provider", name: "anthropic" });
    const snapshot = { providers: [], models: [], specialists: [], tools: [], registry_path: "/tmp/registry.json" };
    vi.mocked(listEntities).mockResolvedValue(snapshot);

    await loadRegistry();

    expect(get(selection)).toBeNull();
  });

  it("loadRegistry preserves selection when the selected entity is still present", async () => {
    selection.set({ kind: "model", name: "claude" });
    const snapshot = { providers: [], models: ["claude"], specialists: [], tools: [], registry_path: "/tmp/registry.json" };
    vi.mocked(listEntities).mockResolvedValue(snapshot);

    await loadRegistry();

    expect(get(selection)).toEqual({ kind: "model", name: "claude" });
  });
});

describe("watchRegistry", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    registry.set(null);
    selection.set(null);
  });

  it("calls loadRegistry when the registry-changed event fires", async () => {
    const snapshot = { providers: ["anthropic"], models: [], specialists: [], tools: [], registry_path: "/tmp/registry.json" };
    vi.mocked(listEntities).mockResolvedValue(snapshot);

    // Capture the handler passed to listen and resolve with a spy unlisten.
    const unlisten = vi.fn();
    let capturedHandler: (() => void) | undefined;
    vi.mocked(listen).mockImplementation(async (_event, handler) => {
      capturedHandler = handler as () => void;
      return unlisten;
    });

    const cleanup = await watchRegistry();

    // Simulate the backend emitting "registry-changed".
    expect(capturedHandler).toBeDefined();
    capturedHandler!();

    // Wait for the async loadRegistry inside the handler to resolve.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(listEntities).toHaveBeenCalled();

    cleanup();
    expect(unlisten).toHaveBeenCalled();
  });

  it("returns a cleanup that removes the focus listener", async () => {
    vi.mocked(listen).mockResolvedValue(() => {});

    const addSpy = vi.spyOn(window, "addEventListener");
    const removeSpy = vi.spyOn(window, "removeEventListener");

    const cleanup = await watchRegistry();
    const focusHandler = addSpy.mock.calls.find((c) => c[0] === "focus")?.[1];
    expect(focusHandler).toBeDefined();

    cleanup();
    expect(removeSpy).toHaveBeenCalledWith("focus", focusHandler);
  });
});
