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
# inputs:

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
in-band failure field. It is *not* the `declined` case in §4.

## 4. Headless runs: recover with `else`

A workflow does not always have an interactive front-end. Invoked in CI, driven
by an outer runner through `$SP_OUTPUT_PATH`
([CLI → run workflow](cli.md#run-workflow)), or otherwise non-interactively,
there is simply no one to ask. Mirroring the way
[`run tool` recovers a non-zero exit](workflow-dsl.md#66-invocation-run-kind),
`ask` can carry an `else` block that turns "couldn't ask" into a value instead
of aborting the workflow:

```
city = ask "Which city should I check?" else {
  unavailable: "Portland",
  declined: "Portland",
  _: "Portland"
}
```

The arms are keyed by a **fixed, built-in vocabulary** of reasons — unlike a
tool's `else`, whose arm names come from that tool's `# exits:` header, the set
here is closed and the same for every `ask`:

| reason | when it fires |
| --- | --- |
| `unavailable` | the run has no interactive front-end (headless: CI, `$SP_OUTPUT_PATH`-driven, piped, etc.), so the question can't be put to anyone. |
| `declined` | a front-end **is** present, but the user dismisses or cancels the prompt, or closes input (EOF) without answering. |
| `_` | default — catches any reason not named above. |

The rules match [`run tool`'s `else`](workflow-dsl.md#66-invocation-run-kind):

- **Every arm — and the answered path — produces a `string`.** Because `ask` is
  always a string (§3), this just means each arm is a string expression, so
  `city` above is a `string` no matter which path is taken.
- The block must be **exhaustive**: name both `unavailable` and `declined`, or
  supply `_`.
- With **no** `else` block, an un-answerable `ask` aborts the workflow — the same
  default a non-zero tool exit has when it carries no `else`.

## 5. Where it sits in "errors are data"

`ask` splits the same way the rest of the DSL does
([§7](workflow-dsl.md#7-errors-are-data)):

- **Out-of-band** — *couldn't ask at all* (`unavailable`/`declined`). Like a
  non-zero tool exit: recovered with `else`, or it aborts.
- **In-band** — *the content of the answer*, including an empty string. Ordinary
  string data; branch on it with `if`.
