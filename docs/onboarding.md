# Onboarding: the entity map

This is the orientation doc for someone joining the project. It exists to give
you the *whole* mental model up front, because the rest of the docs each go deep
on one slice and assume you already know where that slice sits.

Read this once, then use the deep-dive links when you need them.

## Three axes, not one list

Everything in `spawningpool` is easier to hold if you stop reading it as a flat
list of concepts and instead see **three orthogonal axes**:

1. **Definition nouns** — the things a user creates, and that reference each
   other. *What gets defined.*
2. **Operations (verbs)** — what you do *to* those nouns. *How you act on them.*
3. **Runtime channels** — how information moves while something runs. *How a run
   communicates.*

A given concept lives on exactly one axis. The spine connecting axis 1 to a
running program is the **two-layer lowering** (below): every definition noun is
inert template data that *compiles down* into a separate runtime vocabulary the
client actually executes.

```
axis 1: providers · models · specialists · tools · workflows   (definition nouns)
axis 2: define · list · show · delete · run                      (operations)
axis 3: data · asks · logs                                       (runtime channels)

           definitions  ──lower──▶  runtime/wire types  ──run──▶  channels
```

---

## Axis 1 — Definition nouns

The five things a user creates. Each references the previous **by name**; that
reference graph is what `define` validates and `delete` warns about.

```
provider   a wire protocol + endpoint + API-key env var   (Anthropic, or a local LM Studio)
  └─ model       an API id + token limits, under a provider
       └─ specialist   a system prompt + tools, on a model
tool       an executable script a specialist (or workflow) may call
workflow   a DSL script chaining tools + specialists with typed data
```

Source: `spawningpool/src/domain.rs` (`ProviderDef`, `ModelDef`, `Specialist`,
`ToolDef`, `Registry`). The full create-it-yourself walkthrough is the
[Quickstart](README.md#quickstart).

> **Sharp edge.** The internal `EntityKind` enum has only **four** variants —
> `Provider`, `Model`, `Tool`, `Specialist`. Workflows aren't in it, and tools
> are a *derived view* of a script rather than stored data. Axis 3 (storage)
> explains why; don't assume "five nouns" means "five symmetric records."

---

## The two-layer lowering (the spine)

This is the concept most likely to trip you, because it's invisible from the
noun list alone. There are **two parallel vocabularies**:

- **The definition layer** (`domain.rs`) — plain, serializable templates.
  Nothing here talks to a provider or the network.
- **The runtime / wire layer** (`spawningpool/src/ai/`) — what the client
  actually sends and receives.

A run *lowers* the first into the second. The bridges are small and worth
knowing by name:

| Definition (axis 1) | lowers via | Runtime / wire type |
| --- | --- | --- |
| `ModelDef` + its `ProviderDef` | `ModelDef::resolve` / `Registry::resolve_model` | `Model` (id, api, base_url, limits) |
| `Specialist` + prompt + tools | `run::build_context` | `Context` (system, `Message`s, `Tool`s) |
| `ToolDef` | `ToolDef::to_tool` | `Tool` (name, description, JSON-Schema params) |

The runtime layer's vocabulary (in `ai/`, re-exported from `ai::mod`) is:

- **`Model`** — which protocol to speak (`Api`) and where to send it.
- **`Context`** — a system prompt + a list of `Message`s + the exposed `Tool`s.
- **`Message`** — a `Role` (`User` / `Assistant`) plus a list of `ContentBlock`s.
- **`ContentBlock`** — `Text`, `Thinking`, `ToolCall`, or `ToolResult`. All
  message content is an array of these, so interleaved thinking/text/tool-use is
  uniform across providers.
- **`StopReason`** — `Stop`, `Length`, `ToolUse`, `Refusal`, `Error`.
- **`Usage`** — input/output token counts (dollar cost is deliberately *not*
  computed here).

The agentic loop that drives all this lives in `spawningpool/src/run.rs`
(`run_specialist`). When you read it, you'll meet the runtime types cold unless
you've internalized this table — that's the whole reason it's here.

---

## Axis 2 — Operations (verbs)

The noun set tells you *what* exists; the verb set tells you *what you can do*.
They're orthogonal: nearly every verb applies to nearly every noun.

| Verb | What it does |
| --- | --- |
| `define` | create/update a noun |
| `list` | enumerate nouns of one kind |
| `show` | print one noun's full definition |
| `delete` | remove a noun (warns about referrers it would orphan) |
| `run` | execute a runnable |
| `tui` | browse and manage everything interactively |

`run` has its own small sub-set — **the runnables** — selected by target:

```
run specialist <name> --prompt ...      one specialist against a prompt
run tool <name> --arg KEY=VALUE ...      one tool script directly
run workflow <name> --arg KEY=VALUE ...  a DSL workflow
```

These three map 1:1 onto the DSL's `run specialist` / `run tool` /
`run workflow` constructs, so the same three runnables exist whether you invoke
from the shell or from inside a workflow. Source: `cli/src/cli.rs` (`Command`,
`RunTarget`); full flag reference in the [CLI docs](cli.md).

---

## Axis 3 — Storage, and the registry-vs-file split

The nouns don't all live in the same place, and the asymmetry is load-bearing.

| Noun | Where it lives | Form |
| --- | --- | --- |
| provider, model, specialist | `registry.json` | a JSON record in the `Registry` |
| tool | `tools/` folder | an executable script (the `ToolDef` is *derived* by reading its header) |
| workflow | `workflows/` folder | a `.spool` DSL source file |

So **three nouns are registry-backed and two are file-backed**. That's why
`EntityKind` lists four kinds (workflows are addressed by filename, not tracked
in the registry) and why a tool is a "view" rather than stored data — its
description and params are parsed from the script's `# desc:` / `# params:`
header each time.

Locations (`spawningpool/src/store.rs`):

- Registry: `$SPAWNINGPOOL_REGISTRY`, else `$SPAWNINGPOOL_HOME/registry.json`,
  else `~/.spawningpool/registry.json`. A missing file is an empty registry.
- `tools/` and `workflows/` are siblings of the registry file.

Writes are atomic (temp file + rename) but assume a **single writer** — see the
concurrency note in `store.rs`. Full details in [Configuration](configuration.md).

---

## The DSL type system (the shape language for data)

Axis 3's **data** channel is *typed*, and this is the vocabulary those types are
written in. It's a small, closed grammar (`spawningpool/src/types.rs`,
[Workflow DSL §2](workflow-dsl.md)):

| Type | Notation |
| --- | --- |
| string | `string` |
| number | `number` |
| bool | `bool` |
| array | `[T]` |
| object | `{ "k": T, "k2": T2 }` (listed keys required and exhaustive) |

Supporting records:

- **`Param`** — a name + a `Type`. A bare header param (no `:type`) is `string`.
  Used by both tool `# params:` headers and workflow `# inputs:`.
- **`ExitCode`** — one `# exits:` entry: a numeric `code`, a DSL-identifier
  `name` a workflow's `else` arm can branch on, and an optional human `desc`.

Every `Type` lowers to JSON Schema via `Type::to_schema` — the *same* schema the
tool-call validator and `ToolDef::to_tool` consume, so the data channel, tool
parameters, and validation all speak one notation. That's the connective tissue
to the [data-flow contracts](data-flow.md).

---

## Axis 3 — Runtime channels (recap)

Already documented in depth; here only to close the map. Every run moves
information along **three distinct channels**:

| Channel | Carries | Read back by the run? | Deep dive |
| --- | --- | --- | --- |
| **data** | typed values, step → step | yes — the next step consumes it | [Data flow](data-flow.md) |
| **asks** | a question out, an answer back | yes — the answer re-enters as data | [Asking the user](ask.md) |
| **logs** | a record of what happened | no — purely outward | [Workflow logging](workflow-logging.md) |

The one-map-over-three is [The three channels](channels.md). The short version:
data and asks are *part of the computation*; logs only *report on it*.

---

## Where to go next

- Build something end to end: [Quickstart](README.md#quickstart).
- Write a tool: [Writing tools](tools.md).
- Chain things: [Workflow DSL](workflow-dsl.md), then [Data flow](data-flow.md).
- Understand a run's information flow: [The three channels](channels.md).
