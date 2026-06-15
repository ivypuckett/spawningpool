# Workflow DSL (v1)

A minimal language for orchestrating tools and specialists. It is a *conceptual*
(not strict) superset of JSON: JSON values are the data, and the DSL adds typed
declarations, expressions, selection, iteration, tool calls, and specialist
calls on top.

This is v1 and deliberately small. Deferred to later: the forward-pipe operator,
fan-out/parallelism, functions, macros, and boolean math. Types are **trusted**
from headers in v1 — there is no runtime verification that a tool's output
matches its declared type.

## 1. Why this exists

Previously the project punted orchestration to bash (see `README.md` →
Simplifications). That is no longer true: chaining specialists with shared,
typed, structured data is awkward in bash, so the project now ships a small
orchestrator. The README's "bash is sufficient" simplification is replaced by a
pointer to this document.

## 2. Types

Types are known at script-execution time because every tool and specialist
declares its input and output types in its header. The type grammar:

| Type | Notation | Notes |
| --- | --- | --- |
| string | `string` | |
| number | `number` | JSON number (int or float) |
| bool | `bool` | |
| array | `[T]` | every element is `T` |
| object | `{ "k": T, "k2": T2 }` | the listed keys are **required and exhaustive** |

**Force everything to be declared (v1).** An object type lists every key the DSL
will let you access. Extra keys present at runtime are invisible to the
type-checker and cannot be accessed. There is no `any`/unknown type in v1.

### 2.1 Lowering to JSON Schema

This notation lowers to JSON Schema so the existing validator
(`spawningpool::ai::validation::validate_tool_call`) and the schema builder in
`ToolDef::to_tool` (`spawningpool/src/domain.rs`) are reused rather than
duplicated:

- `string`/`number`/`bool` → `{"type": "string" | "number" | "boolean"}`
- `[T]` → `{"type": "array", "items": <T>}`
- `{ "k": T, ... }` → `{"type": "object", "properties": {...}, "required": [<all keys>]}`

`ToolDef::to_tool` today maps every bare param to a required string; the typed
form is the same builder with the element type taken from the header instead of
hard-coded to `string`. A bare, untyped param keeps meaning `string`, so
existing tool headers are unchanged.

## 3. Tool headers (extended)

Tools stay executable scripts with a comment header (see `docs/tools.md`). Two
additions:

```sh
#!/bin/sh
# desc: Look up a host's latency
# params: HOST:string, COUNT:number
# output: { "host": string, "reachable": bool, "ms": number }
echo "pinging $HOST x$COUNT"          # normal logging -> stdout/stderr, as today
printf '{"host":"%s","reachable":true,"ms":12}' "$HOST" > "$SP_OUTPUT_PATH"
```

- `# params:` gains optional `:type` suffixes. No suffix means `string`
  (backward compatible). Parsing extends `parse_header` in
  `spawningpool/src/script.rs`.
- `# output:` declares the tool's output type using the notation in §2.
- **Output is read GHA-style.** Before running a tool, the runner sets
  `SP_OUTPUT_PATH` to a fresh temp file. The tool writes its structured result
  there as JSON. After the tool exits, the runner reads and parses that file as
  the tool's declared output type. stdout/stderr remain ordinary logs (combined
  as today) and are **not** parsed as output.
- Inputs are passed as environment variables exactly as today; non-string values
  are passed as their JSON text (already documented in `docs/tools.md`).

## 4. Specialist calls and the unified return type

`ask <specialist> <prompt-expr>` runs a specialist. Both constrained and
unconstrained specialists return the **same** shape — the existing
`run --output json` envelope (`cli/src/main.rs`). As a DSL type:

```
{
  "output": string,
  "thinking": string,
  "inputTokens": number,
  "outputTokens": number,
  "stopReason": string,
  "model": string,
  "specialist": string,
  "turns": number,
  "toolCalls": [ { "name": string, "success": bool, "output": string } ]
}
```

- An **unconstrained** specialist's answer is in `.output`.
- A **constrained** specialist's forced call is in `.toolCalls` — e.g. read
  `ask classify "..."` as `result.toolCalls.0.output`.

This means no new return plumbing is needed; the DSL consumes what
`run_specialist` already produces.

## 5. Workflow structure

There are **no `input`/`process`/`output` sections** — input/process/output is
the conceptual shape, not literal syntax. A workflow is just a flat **series of
statements** separated by a **double newline**, and most statements are
assignments.

```
CITY: string

RETRIES: number

weather = call get_weather { CITY: CITY, RETRIES: RETRIES }

summary = ask reporter ("Summarize: " + weather.summary)

result = { "city": CITY, "ok": weather.reachable, "report": summary.output }
```

- **Input** is declared with a typed statement `ENVKEY: jsonType` (e.g. `CITY:
  string`). Like tool params, inputs arrive from the environment.
- **Process** is the run of assignment/call/control-flow statements that follow.
- **Output** — the value the workflow produces, written to its own
  `$SP_OUTPUT_PATH` so a workflow is itself composable as a tool. How that value
  is designated (final statement vs. a dedicated form) is an open decision; see
  §8.

The common idiom is to **assign** a control-flow expression rather than write it
bare: `var = if (...) ..., (_) ...` and `var = foreach [item: arr] (...)`. The
raw `if`/`foreach` syntax is the expression itself (§6.4, §6.5); the `var = …`
form is just that expression on the right of an assignment, which is how it's
normally used since both yield values.

## 6. Statements and expressions

### 6.1 Statement separation

There are **no block delimiters**. Statements are separated by a double newline.
A control-flow construct's body is a **single sub-expression**; nesting is
achieved because that sub-expression can itself be another control-flow
construct.

### 6.2 Assignment

```
name = expr
```

`name` is bound for the rest of the workflow. (Iteration introduces a local
binding; see §6.5.)

### 6.3 Operators

- Logical: `||`, `&&`, `!`
- Arithmetic: `+`, `-`, `*`, `/`, `%`, `^` (where `^` is power)
- **No operator precedence.** Evaluation is strictly left-to-right; use
  parentheses to group. So `(1 + 2 - 3 / 4 % 5 ^ 6)` evaluates left to right.
- Grouping is parentheses only.

### 6.4 Selection (if)

```
if (expr) result, (expr) result, ..., (_) result
```

The first branch whose condition is truthy yields its result. **A selection must
always end with the `(_)` default branch.** No per-branch variable binding in v1.
This is an expression; the common form is `var = if (...) ..., (_) ...` (§5).

### 6.5 Iteration (for)

```
for [item: array] (expr)
```

Binds `item` to each element of `array` and evaluates the single sub-expression
per element; the iteration's value is the array of results (map semantics —
"monadic uses map"). The body accepts **one** sub-expression; if that
sub-expression is itself a `for`, it likewise accepts one sub-expression, and so
on for deeper nesting. This is an expression; the common form is `var = foreach
[item: arr] (...)` (§5).

### 6.6 Tool call

```
call <toolname> <map-expr>
```

The single argument is a map supplying the tool's declared params by name
("monadic uses map"). The call's value is the tool's declared `# output:` type,
read from the tool's `$SP_OUTPUT_PATH` after it runs.

### 6.7 Specialist call

```
ask <specialist> <prompt-expr>
```

`prompt-expr` evaluates to the user prompt (a string). The call's value is the
unified envelope of §4.

### 6.8 Access

Access into arrays and objects uses `.`:

- `var.key` — bare identifier is a **literal key** (not a variable).
- `var."key"` — quoted literal key (for keys that aren't bare identifiers).
- `var.0` — array index (requires `var` to be an array type).
- `var.(expr)` — computed access; `expr` is evaluated, then used as index/key.

Access is type-directed:

- A literal/quoted key requires an object type that **declares** that key, and
  yields that key's declared type.
- An index requires an array type and yields its element type.
- Computed `var.(expr)` is allowed on **arrays** (yields the element type). On
  objects it is allowed only when all declared value types are identical (so the
  result type is statically known); otherwise it is a v1 error. See open
  decisions.

## 7. Errors are data

Runtime failures are values, not aborts. The workflow does not throw on a failed
call:

- A tool encodes failure in its declared `# output:` type (e.g. a `reachable:
  bool` field). The DSL does not branch on a tool's exit code; orchestration
  decisions are made on the structured output.
- A specialist's outcome (including `stopReason` and per-call `success`) is part
  of its returned envelope (§4), so callers branch on it explicitly.

## 8. Open decisions (need a call before implementation)

These are intentionally not pinned yet; each has a recommended default:

1. **Workflow invocation / CLI surface, and how the output value is designated.**
   How a workflow is defined and run (e.g. `spawningpool run --workflow <file>`),
   whether workflows live beside tools so they nest uniformly, and how a workflow
   names the value written to `$SP_OUTPUT_PATH` — the value of the final statement,
   or a dedicated form. *Recommended:* treat a workflow as a tool-like artifact
   with typed inputs (env) and `$SP_OUTPUT_PATH` output, runnable via a new
   `run --workflow` flag, with the last statement's value as the output.
2. **Exit-code semantics for tools in a workflow.** With "errors are data," a
   non-zero exit is informational. *Recommended:* the runner records the exit
   code but does not abort; if the tool wrote no output file, that is a runtime
   error surfaced as a workflow failure (since v1 trusts the header to declare an
   output).
3. **Computed access into heterogeneous objects** (§6.8). *Recommended:* reject
   in v1 (require uniform value types or an array), revisit when an `any` type
   lands.
4. **Truthiness** for `if` conditions and logical ops on non-bool values.
   *Recommended:* require `bool` in v1 (no implicit coercion), consistent with
   "force everything to be declared."
