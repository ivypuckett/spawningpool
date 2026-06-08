import { writable } from "svelte/store";
import type { RegistrySnapshot, Selection } from "./types";
import { listEntities } from "./api";

/** The latest registry snapshot, or null before the first load. */
export const registry = writable<RegistrySnapshot | null>(null);

/** The currently-selected entity in the UI, or null when nothing is selected. */
export const selection = writable<Selection | null>(null);

/** Fetch the registry snapshot and publish it to the `registry` store. */
export async function loadRegistry(): Promise<void> {
  registry.set(await listEntities());
}
