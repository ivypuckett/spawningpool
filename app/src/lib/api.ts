import { invoke } from "@tauri-apps/api/core";
import type { RegistrySnapshot } from "./types";

/** Load the registry snapshot (names of every defined entity) from the backend. */
export function listEntities(): Promise<RegistrySnapshot> {
  return invoke<RegistrySnapshot>("list_entities");
}
