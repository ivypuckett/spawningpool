# Human in the loop (`converse`)

A workflow is a straight-line pass that runs start to finish (see
[workflow-dsl.md](workflow-dsl.md) §5) — it has no loop and no "wait for input"
construct, on purpose. Turn-taking with a human lives **outside** the DSL, in
the `converse` runner. The runner owns three things the language deliberately
doesn't:

1. the **loop** (keep taking turns),
2. the carried **conversation window** (persisted per run, so it survives across
   turns and process restarts), and
3. the **`continue`** exit.

Each turn the runner re-invokes a **one-turn workflow** — a pure function of its
inputs. All continuity is the `window` string the runner threads back in.

## The three modes

Each turn you pick a mode:

| Mode | Shortcut | What it does |
| --- | --- | --- |
| `discuss` | `d` | Take another turn: your message is added to the window and answered. |
| `summarize` | `s` | Condense the window into a cohesive, smaller window, then ask what's next. |
| `continue` | `c` | Stop turn-taking. Handled entirely by the runner — it never reaches the workflow. |

`continue` is the runner deciding to stop looping, which is exactly "continue
exits turn-taking". That's why the workflow below only dispatches `discuss` vs.
everything else.

## The contract

The runner supplies exactly these inputs, each a `string`:

- `MODE` — the mode picked this turn: `discuss` or `summarize`.
- `MESSAGE` — your message (used by `discuss`).
- `WINDOW` — the window carried in (`""` on the first turn).

and the workflow must return an object with two `string` fields:

- `window` — the new window to carry into the next turn.
- `reply` — the text to show you.

## The one-turn workflow

This is [`examples/workflows/converse.spool`](../examples/workflows/converse.spool):

```
# inputs: MODE:string, MESSAGE:string, WINDOW:string

prompt = WINDOW + "\n\nUser: " + MESSAGE

turn = if (MODE == "discuss") (run specialist chat prompt),
       (_)                    (run specialist summarizer WINDOW)

result = if (MODE == "discuss")
           { "window": prompt + "\n\nAssistant: " + turn.output,
             "reply":  turn.output },
         (_)
           { "window": turn.output,
             "reply":  turn.output + "\n\nWhat would you like to do next?" }
```

Why it's 0-waste:

- `prompt` is pure string work — cheap, fine to run every turn.
- `turn` is the **only** model call. Because `if` evaluates only the taken
  branch, exactly one of `chat`/`summarizer` runs per turn — never both — and
  its result is bound **once**.
- `result` reuses that single `turn` binding to format the output, with a second
  `if` that does no model calls. This sidesteps the "no per-branch binding" v1
  limitation (workflow-dsl.md §6.4): the expensive call is hoisted to its own
  statement, while the cheap formatting branches freely.

The mode dispatch is the DSL's selection (`if`), made writable by the `==`
equality operator (workflow-dsl.md §6.3).

## Running it

1. Install the example workflow into your workflows folder:

   ```sh
   mkdir -p ~/.spawningpool/workflows
   cp examples/workflows/converse.spool ~/.spawningpool/workflows/
   ```

2. Define the two specialists it calls (a conversational one and a summarizing
   one), e.g.:

   ```sh
   spawningpool define specialist chat \
     --provider <p> --model <m> \
     --system-prompt "You are a helpful conversational assistant."

   spawningpool define specialist summarizer \
     --provider <p> --model <m> \
     --system-prompt "Condense the transcript into a cohesive, self-contained \
   summary, then state the open threads."
   ```

3. Converse:

   ```sh
   spawningpool converse converse
   ```

   ```
   Started run '1718700000-12345'. Pick a mode each turn:
     discuss (d) — take another turn   summarize (s) — condense the conversation   continue (c) — finish

   mode [d/s/c] ▸ d
   you ▸ How should I structure a CLI for a task runner?

   ai ▸ Start with a verb-first command layout: run, list, status...

   mode [d/s/c] ▸ s

   ai ▸ So far: verb-first CLI; config via file with flag overrides.
        What would you like to do next?

   mode [d/s/c] ▸ c

   Conversation saved as run '1718700000-12345'. Resume with:
     spawningpool converse converse --resume 1718700000-12345
   ```

The window is saved after every turn under `~/.spawningpool/runs/<id>.json`, so
`--resume <id>` picks the conversation back up where it left off.
