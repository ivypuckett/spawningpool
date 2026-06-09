import { invoke } from "@tauri-apps/api/core";
import type { EntityKind, RegistrySnapshot } from "./types";

/** Load the registry snapshot (names of every defined entity) from the backend. */
export function listEntities(): Promise<RegistrySnapshot> {
  return invoke<RegistrySnapshot>("list_entities");
}

/** Fetch one entity's full definition (arbitrary JSON shape) from the backend. */
export function showEntity(kind: EntityKind, name: string): Promise<unknown> {
  return invoke("show_entity", { kind, name });
}
