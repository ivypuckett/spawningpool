export type EntityKind = "provider" | "model" | "specialist" | "tool";

export interface RegistrySnapshot {
  providers: string[];
  models: string[];
  specialists: string[];
  tools: string[];
  registry_path: string;
}

export interface Selection {
  kind: EntityKind;
  name: string;
}
