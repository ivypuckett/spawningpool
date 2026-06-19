# Workflow logging

Structured observability for harness refinement, retrospective analysis, and
communicating run state to the user.

## Format: NDJSON

One JSON object per line, emitted in execution order to a log stream (destination
is a deployment decision; the format is independent of it). Every event is
self-contained and parseable with `jq` without context from surrounding lines.
Key order within a line is not significant.

The CLI writes the stream to `logs/<datestamp>-<root>.ndjson` in the working
directory, one file per invocation, where `<root>` is the root workflow name (or
the specialist name for a bare `spawningpool run specialist`). The library itself
is destination-agnostic: it emits events to an injected sink
([`spawningpool::LogSink`]) that stamps each line's `ts` and `run`.

## Universal fields

Every event carries these four fields.

| Field | Type | Notes |
| --- | --- | --- |
| `ts` | string | RFC 3339, millisecond precision — `"2026-06-19T14:23:01.042Z"` |
| `run` | string | 8 hex chars, random per CLI invocation — ties all events from one run together, including sub-workflow events |
| `event` | string | dot-separated type (see below) |
| `wf` | string \| null | workflow name; `null` for a bare `spawningpool run specialist` |

## Events

### Workflow lifecycle

```
workflow.start    inputs: object
workflow.done     elapsed_ms: number
workflow.error    elapsed_ms: number, error: string
```

`workflow.start` fires once per workflow invocation — including sub-workflows
called via `run workflow`. All share the same `run` ID; the `wf` field
distinguishes them.

### Specialist invocations

```
specialist.start  stmt: string|null, specialist: string, model: string, prompt: string
specialist.done   stmt: string|null, specialist: string, stop_reason: string,
                  turns: number, input_tokens: number, output_tokens: number,
                  elapsed_ms: number
```

`stmt` is the workflow variable name being assigned (`null` for a bare specialist
run). A specialist invocation is treated as **atomic** from the workflow's
perspective: `specialist.start` and `specialist.done` bracket the whole
invocation, including all internal turns. Per-turn events are not emitted —
the agentic loop is an implementation detail, not a declarative unit. `turns`
in `specialist.done` is aggregate metadata (the same count that appears in the
result envelope), not a log of state transitions.

`input_tokens` and `output_tokens` are summed across all turns. `stop_reason` is
the run's normalized stop reason, serialized snake_case (`stop`, `length`,
`tool_use`, `refusal`, `error`) — a specialist that settles on a final answer
reports `stop`.

### Tool calls

```
tool.call   stmt: string|null, specialist: string|null, tool: string, args: object
tool.done   stmt: string|null, specialist: string|null, tool: string,
            success: boolean, exit_code: number|null, elapsed_ms: number
```

Covers **all** tool invocations: a `call tool` statement in the workflow
(`specialist: null`) and every tool a specialist chooses to call during its
agentic loop (`specialist: "<name>"`). Tool calls are the finest meaningful
grain — they represent explicit acts regardless of which turn of the loop
triggered them.

`exit_code: null` on a failed `tool.done` means the script never ran to an exit
code: it failed to launch (not executable, missing shebang, or path not found),
the specialist named a tool that doesn't exist, or a signal killed it — as
distinct from a non-zero exit.

### Ask

```
ask.prompt   stmt: string|null, prompt: string
ask.answer   stmt: string|null, answered: boolean
```

`answered: false` means the run was headless (`$SP_OUTPUT_PATH` set or stdin
not a TTY) or the user closed input without replying. The `ask`'s `else`
fallback handles this case; `ask.answer` records that it happened.

## Sub-workflow nesting

A `run workflow` call produces a nested `workflow.start` / `workflow.done` pair
with the same `run` ID and a different `wf`. The outer workflow's `stmt` context
and temporal ordering locate the sub-workflow call in the log without needing a
`parent_wf` field.

## Example

A workflow named `summarise` that fetches a URL directly, then passes the
content to a specialist that calls `search` once before settling:

```json
{"ts":"2026-06-19T14:23:01.042Z","run":"f3a9b21c","event":"workflow.start","wf":"summarise","inputs":{"url":"https://example.com/doc"}}
{"ts":"2026-06-19T14:23:01.055Z","run":"f3a9b21c","event":"tool.call","wf":"summarise","stmt":"raw","specialist":null,"tool":"fetch-url","args":{"url":"https://example.com/doc"}}
{"ts":"2026-06-19T14:23:01.892Z","run":"f3a9b21c","event":"tool.done","wf":"summarise","stmt":"raw","specialist":null,"tool":"fetch-url","success":true,"exit_code":0,"elapsed_ms":837}
{"ts":"2026-06-19T14:23:01.893Z","run":"f3a9b21c","event":"specialist.start","wf":"summarise","stmt":"summary","specialist":"summariser","model":"claude-sonnet-4-6","prompt":"Summarise this: …"}
{"ts":"2026-06-19T14:23:02.103Z","run":"f3a9b21c","event":"tool.call","wf":"summarise","stmt":"summary","specialist":"summariser","tool":"search","args":{"query":"example domain"}}
{"ts":"2026-06-19T14:23:02.313Z","run":"f3a9b21c","event":"tool.done","wf":"summarise","stmt":"summary","specialist":"summariser","tool":"search","success":true,"exit_code":0,"elapsed_ms":210}
{"ts":"2026-06-19T14:23:06.083Z","run":"f3a9b21c","event":"specialist.done","wf":"summarise","stmt":"summary","specialist":"summariser","stop_reason":"stop","turns":2,"input_tokens":2104,"output_tokens":418,"elapsed_ms":4190}
{"ts":"2026-06-19T14:23:06.087Z","run":"f3a9b21c","event":"workflow.done","wf":"summarise","elapsed_ms":5045}
```

The `tool.call` / `tool.done` pair between `specialist.start` and
`specialist.done` is the specialist's tool use — the specialist ran two turns
internally but only the aggregate is logged at that level.
