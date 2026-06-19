# Asking the user (`ask`)

A workflow expression that pauses the run, puts a question to the human
operating it, and resolves to whatever they type back. It is the one point in
the DSL where control returns to a person mid-run.

Unlike [`run tool` / `run specialist` / `run workflow`](workflow-dsl.md#66-invocation-run-kind),
`ask` resolves **no named, on-disk entity** — there is nothing in a folder to
`define`, list, or show. The question is written inline, so `ask` is a built-in
keyword like `if`/`for`/`do`, not a `run <kind> <name>`.

## 1. Surface

```
answer = ask <prompt-expr>
```

- `prompt-expr` evaluates to a `string` — the question shown to the user.
- The value of `ask` is **always a `string`**: the user's reply, verbatim (§3).

```
city = ask "Which city should I check the weather for?"

weather = run tool get_weather { CITY: city }

result = { "city": city, "ms": weather.ms }
```

`ask` yields a `string`, so it can appear anywhere a string expression can —
on its own as above, or as a sub-expression, e.g.
`run specialist reporter ("Summarize weather for " + ask "Which city?")`.

## 2. Workflows only

`ask` exists **only in the workflow DSL**, alongside `if`/`for`/`do`/`run`
([Workflow DSL §6](workflow-dsl.md#6-statements-and-expressions)). A tool is a
non-interactive script and a specialist is a model call — neither has a person
on the other end to turn to — so the question of "ask the user" only arises
while a workflow is orchestrating. There is no `ask` subcommand on the CLI and
no `ask` directive in a tool header.

## 3. Why the answer is always a string

The user types free text, so the only thing `ask` can honestly promise is a
string. v1 does **not** parse or validate that text into a richer type — doing so
would need a re-prompt-on-bad-input loop and a validation contract that v1
doesn't have. A workflow that needs more than text does the conversion
**downstream**, with the tools the DSL already gives it:

- Need a number or a structured value? Pass the answer to a tool that parses it
  (its `# output:` type carries the result), exactly as any other tool output.
- Need one of a fixed set of choices? Branch on the text with
  [`if`](workflow-dsl.md#64-selection-if).

An **empty** answer (the user just presses enter) is still a successful string
answer — the empty string `""`. That is in-band data: test for it with `if`, the
same way [errors-are-data](workflow-dsl.md#7-errors-are-data) treats a tool's
in-band failure field. It is *not* the same as the user declining to answer (§4).

## 4. Headless runs: recover with `else`

A workflow does not always have an interactive front-end. Invoked in CI, driven
by an outer runner through `$SP_OUTPUT_PATH`
([CLI → run workflow](cli.md#run-workflow)), or otherwise non-interactively,
there is simply no one to ask. An `ask` can carry an `else` fallback that turns
"couldn't ask" into a value instead of aborting the workflow:

```
city = ask "Which city should I check?" else "Portland"
```

`else <string-expr>` is a **single fallback string**, used **whenever the
question can't be answered** — for any reason: the run is headless (no front-end:
CI, `$SP_OUTPUT_PATH`-driven, piped, etc.), or a front-end is present but the
user cancels or closes input (EOF) without answering. There is no per-reason
branching — unlike [`run tool`'s `else`](workflow-dsl.md#66-invocation-run-kind),
which keys arms off a tool's declared exits. Because `ask` always yields a
`string` (§3), the recovery is just another string, so one expression is all the
fallback needs. (If a workflow ever needs to know *why* it fell back, that's a
richer exit-envelope story left for later, the same as for tools.)

- The fallback must be a `string` — the same type the answer would have had — so
  `city` is a `string` whichever path is taken.
- With **no** `else`, an un-answerable `ask` aborts the workflow — the same
  default a non-zero tool exit has when it carries no `else`.

## 5. Where it sits in "errors are data"

`ask` splits the same way the rest of the DSL does
([§7](workflow-dsl.md#7-errors-are-data)):

- **Out-of-band** — *couldn't ask at all* (headless, or the user declined). Like
  a non-zero tool exit: recovered with the `else` fallback, or it aborts.
- **In-band** — *the content of the answer*, including an empty string. Ordinary
  string data; branch on it with `if`.
