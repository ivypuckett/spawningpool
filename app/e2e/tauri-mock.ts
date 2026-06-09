// Installs a minimal in-page mock of Tauri's IPC bridge so the Svelte frontend
// runs in a plain browser (no webview, no Rust backend) during a screenshot render.
//
// The real `@tauri-apps/api` talks to the backend purely through two globals:
//   - window.__TAURI_INTERNALS__.invoke / .transformCallback
//   - window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener
// (see node_modules/@tauri-apps/api/core.js and event.js). We implement just
// those so `invoke("list_entities")`, `invoke("show_entity", …)`, and the
// `listen("registry-changed", …)` subscription all resolve against `seed`.

import type { BrowserContext } from "@playwright/test";
import type { Seed } from "./seed";

// Runs in the browser before any app code. Kept as a standalone function so
// Playwright can serialize it into an init script with `seed` as its argument.
function mockBridge(seed: Seed): void {
  const callbacks = new Map<number, (payload: unknown) => void>();
  let nextId = 1;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__TAURI_INTERNALS__ = {
    transformCallback(cb: (payload: unknown) => void): number {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number): void {
      callbacks.delete(id);
    },
    async invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
      if (cmd === "list_entities") {
        return seed.snapshot;
      }
      if (cmd === "show_entity") {
        const kind = args?.kind as keyof Seed["definitions"];
        const name = args?.name as string;
        const def = seed.definitions[kind]?.[name];
        if (def === undefined) {
          throw new Error(`no such ${kind} ${name}`);
        }
        return def;
      }
      // Event plugin: registering a listener returns an opaque id. We never
      // emit, so the registry simply loads once — perfect for a deterministic
      // render. Everything else (unlisten, emit) is a no-op.
      if (cmd === "plugin:event|listen") {
        return nextId++;
      }
      return null;
    },
  };

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(): void {},
  };
}

/** Inject the Tauri IPC mock into every page in `context` before it loads. */
export function installTauriMock(
  context: BrowserContext,
  seed: Seed,
): Promise<void> {
  return context.addInitScript(mockBridge, seed);
}
