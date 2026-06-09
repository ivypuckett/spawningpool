import { writable } from "svelte/store";
import type { RegistrySnapshot, Selection } from "./types";
import { listEntities } from "./api";
import { listen } from "@tauri-apps/api/event";

/** The latest registry snapshot, or null before the first load. */
export const registry = writable<RegistrySnapshot | null>(null);

/** The currently-selected entity in the UI, or null when nothing is selected. */
export const selection = writable<Selection | null>(null);

/** Fetch the registry snapshot and publish it to the `registry` store. */
export async function loadRegistry(): Promise<void> {
  registry.set(await listEntities());
}

/** Reload the registry whenever the backend signals a change, and on window focus.
 *  Returns a cleanup function that removes both listeners. */
export async function watchRegistry(): Promise<() => void> {
  const unlisten = await listen("registry-changed", () => {
    loadRegistry().catch(console.error);
  });
  const onFocus = () => {
    loadRegistry().catch(console.error);
  };
  window.addEventListener("focus", onFocus);
  return () => {
    unlisten();
    window.removeEventListener("focus", onFocus);
  };
}
