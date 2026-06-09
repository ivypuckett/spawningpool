// Deterministic registry data the mocked Tauri backend serves during a
// screenshot render. The shapes mirror what the real `list_entities` /
// `show_entity` commands return (see app/src-tauri/src/commands/registry.rs), so
// the rendered UI looks exactly like it does against a populated registry on disk.

import type { EntityKind, RegistrySnapshot } from "../src/lib/types";

export interface Seed {
  snapshot: RegistrySnapshot;
  // definitions[kind][name] is whatever `show_entity(kind, name)` returns.
  definitions: Record<EntityKind, Record<string, unknown>>;
}

export const seed: Seed = {
  snapshot: {
    providers: ["anthropic", "openai"],
    models: ["claude-opus-4", "claude-3-5-sonnet", "gpt-4o"],
    specialists: ["code-reviewer", "summarizer"],
    tools: ["ping", "web-search"],
    registry_path: "/home/agent/.config/spawningpool/registry.json",
  },
  definitions: {
    provider: {
      anthropic: {
        name: "anthropic",
        api: "anthropic-messages",
        base_url: "https://api.anthropic.com",
        api_key_env: "ANTHROPIC_API_KEY",
        constrained_decoding: false,
      },
      openai: {
        name: "openai",
        api: "openai-completions",
        base_url: "https://api.openai.com/v1",
        api_key_env: "OPENAI_API_KEY",
        constrained_decoding: true,
      },
    },
    model: {
      "claude-opus-4": {
        id: "claude-opus-4",
        name: "Claude Opus 4",
        provider: "anthropic",
        max_tokens: 8192,
        context_window: 200000,
      },
      "claude-3-5-sonnet": {
        id: "claude-3-5-sonnet",
        name: "Claude 3.5 Sonnet",
        provider: "anthropic",
        max_tokens: 8192,
        context_window: 200000,
      },
      "gpt-4o": {
        id: "gpt-4o",
        name: "GPT-4o",
        provider: "openai",
        max_tokens: 16384,
        context_window: 128000,
      },
    },
    specialist: {
      "code-reviewer": {
        name: "code-reviewer",
        provider: "anthropic",
        model: "claude-opus-4",
        system_prompt:
          "Review the diff for correctness bugs and surface concrete, actionable findings.",
        tools: ["web-search"],
        constraint: null,
        reasoning: "high",
        stream: true,
      },
      summarizer: {
        name: "summarizer",
        provider: "openai",
        model: "gpt-4o",
        system_prompt: "Summarize the input in three sentences or fewer.",
        tools: [],
        constraint: null,
        reasoning: "off",
        stream: false,
      },
    },
    tool: {
      ping: {
        name: "ping",
        script: "/home/agent/.config/spawningpool/tools/ping",
        description: "Check whether a host is reachable.",
        params: ["HOST"],
      },
      "web-search": {
        name: "web-search",
        script: "/home/agent/.config/spawningpool/tools/web-search",
        description: "Search the web and return the top results.",
        params: ["QUERY", "LIMIT"],
      },
    },
  },
};
