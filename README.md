# Spawning Pool

Goal: create hyper-specific agents with minimal system prompts that do one thing and one thing well.

> **⚠️ Terminology conflict — read first.** This design doc uses **"specialist"**
> throughout. The actual code uses **"expert"** everywhere — the type is
> `Expert`, the CLI noun is `expert`, the flags are `--expert`, and there are no
> `specialist`/`lenny`/`ling` aliases. Every "specialist" below maps to an
> "expert" in the implementation. The conflicts are flagged inline in
> `> **Status:**` callouts.

## Requirements

- Creating/updating specialists needs to be trivial.
- Need to be able to give specialists tools easily.
- Need to be able to evaluate specialists against each other quickly.
- Needs to be written such that a specialist *could* define and refine other specialists dynamically.

> **Status:** The first two are met via `sp define expert` / `sp define tool`.
> Cross-specialist *evaluation* is not implemented. Dynamic self-definition is
> not implemented: `define` is a CLI subcommand, not a model-callable tool, so an
> expert cannot yet define other experts at runtime (see *Default tools* below).

## Terms

- **Tool:** deterministic, callable thing which a model can utilize.
- **System prompt:** standard definition.
- **Specialist:** the template of model provider, model, system prompt, and tools which can be instantiated and called with a user prompt.

> **Status:** Implemented as `Expert` (`spawningpool/src/domain.rs`). The runtime
> template additionally carries `reasoning` (off|low|medium|high) and `stream`
> fields not mentioned here.

## Default tools

- Define specialist
- Define tool (cli to tool call definition)

> **Status — not implemented as tools.** "Define specialist" and "define tool"
> exist only as the CLI subcommands `sp define expert` / `sp define tool`. They
> are not exposed to a model as callable tools, so no expert currently ships with
> default tools. The only tools an expert can call are ones defined from Taskfile
> tasks via `sp define tool`.

## CLI

The binary is named `spawningpool`; its CLI name is `sp`.

```bash
# run (alias: spawn)
sp run \
  --specialist name \
  --prompt 'prompt'

# specialist (alias: lenny, ling)

# define
sp define specialist name \
  --systemPrompt 'prompt' \
  --provider provider \
  --model model \
  --tools 'tool,tool2' \
  --constraint 'tool'

sp define provider name
sp define model name

# delete
sp delete specialist name
sp delete model name
sp delete provider name
sp delete tool name

# list
sp list specialists
sp list models
sp list providers
sp list tools
```

> **Status — the actual CLI differs.** What is implemented today
> (`cli/src/main.rs`):
>
> ```bash
> # run (alias: spawn) — note --expert, not --specialist
> sp run --expert name --prompt 'prompt'
>
> # define expert — flag is --system-prompt (kebab-case), plus --reasoning/--stream
> sp define expert name \
>   --provider provider \
>   --model model \
>   --system-prompt 'prompt' \
>   --tools 'tool,tool2' \
>   --constraint 'tool' \
>   --reasoning off \
>   --stream
>
> # define provider — requires --api and --base-url, not just a name
> sp define provider name --api anthropic|openai --base-url URL [--api-key-env ENV]
>
> # define model — keyed by id, requires a provider and limits
> sp define model id --provider provider --max-tokens N --context-window N [--name NAME]
>
> # define tool — sourced from a Taskfile task
> sp define tool name --taskfile PATH --task TASK
>
> # delete / list use the noun "expert"
> sp delete expert|provider|model|tool name
> sp list experts|providers|models|tools
> ```
>
> There are no `lenny`/`ling` aliases, and there is no `sp define provider name`
> / `sp define model name` short form — both require their flags. The registry is
> persisted to `$SPAWNINGPOOL_HOME/registry.json` (default
> `~/.spawningpool/registry.json`), overridable with `$SPAWNINGPOOL_REGISTRY`.

## Technology choices

1. **Rust:** Catches the most bugs at compile time.
2. **Scope:** easiest task management platform available.
3. **Taskfiles:** allows us to not reinvent the wheel for tool calls and the standard. Parsing of variables means we can read these deterministically. Also used at the root level of the project.
4. **models.dev:** provides the json schemas for providers and models.
5. **Clap:** cli parsing. Necessary for first version, complete integration testing, AND model calling.
6. **Tauri:** Visual app. Necessary past version one when it's ready to be used by humans.
7. **Lucifer:** cli integration testing application.
8. **task-keeper:** already parses Taskfiles, inspires our implementation of task file parsing.

> **Status — what's actually wired in:**
>
> 1. **Rust** ✅ — workspace of `spawningpool` (lib) + `cli` (binary).
> 2. **Scope** ❌ — not present in the repo; no task-management integration.
> 3. **Taskfiles** ⚠️ — consumed *only* as tool-definition sources
>    (`spawningpool/src/taskfile.rs`). There is **no root Taskfile**; build/test
>    is plain `cargo` (see `CLAUDE.md`).
> 4. **models.dev** ❌ — **directly contradicted.** `ai/catalog.rs` states
>    spawningpool *deliberately does not* embed a catalog of hosted models or
>    their limits; models live in your own registry, defined via `sp define
>    model`. No models.dev schemas are used.
> 5. **Clap** ✅ — clap v4 (derive) drives the CLI.
> 6. **Tauri** ❌ — not a dependency (explicitly post-v1).
> 7. **Lucifer** ❌ — not used; tests are standard `cargo test`, including
>    `spawningpool/tests/ai_integration.rs`.
> 8. **task-keeper** ⚠️ — inspiration only, not a dependency. Taskfile parsing is
>    hand-rolled with `serde_yaml` + `regex`.
>
> **Providers actually implemented:** `anthropic-messages` (Claude) and
> `openai-completions` (LM Studio / any OpenAI-compatible endpoint), selected at
> runtime from the model's `api`.

Why not use the pi sdk? It's awesome but does not support constrained decoding. That is a core feature for any specialist whose sole job is to figure something out and call a tool with the results.

> **Status:** Constrained decoding is realized as a forced tool choice — the
> expert's `--constraint <tool>` becomes `tool_choice` — plus an opt-in
> tool-call argument validator (`ai/validation.rs`). The pi SDK is not a
> dependency.

## Phases

1. Define and document providers.json (decide whether to steal pi or opencode and just use that)
2. Install above technologies into a hello world project
3. `sp list providers`, `models`
4. `sp define provider`
5. `sp list specialists`
6. `sp define model`
7. `sp define specialist`
8. `sp run specialist`
9. `sp list tools`
10. `sp define tool`
11. `sp delete *`

> **Status:** Phases 3–11 are implemented as `sp` subcommands (using the noun
> `expert`, not `specialist`). Phase 1 is **stale/contradicted**: there is no
> `providers.json` and no pi/opencode import — providers are defined via the CLI
> and persisted into `registry.json`. This also conflicts with the *models.dev*
> claim above; the implemented design is "no embedded provider/model catalog."

## Simplifications

- A chain is a sequence/selection/iteration of specialists. Bash and scripting allows for this and provides an existing abstraction/entry point. The project does not prioritize shared memory. Therefore, bash is sufficient. If better tooling is desired, LangGraph and Mastra have both invested in this space and could be used alongside it.

## Build & test

```sh
cargo build      # build the workspace
cargo test       # run all tests
cargo clippy     # lint
cargo fmt        # format
```

The binary is built to `target/debug/spawningpool` (or `target/release/spawningpool`).
See `CLAUDE.md` for contributor rules and git-hook setup.
