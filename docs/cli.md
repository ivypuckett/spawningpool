# CLI reference

The binary is `spawningpool`. Install it with `cargo install spawningpool-cli`
(see the [Quickstart](README.md#quickstart)).

Every command operates on one JSON registry on disk. A missing registry is
treated as empty, so the first `define` creates it. See
[Configuration](configuration.md) for where it lives.

- [`spawningpool`](#spawningpool-bare) — show the next step
- [`spawningpool define`](#spawningpool-define) — create entities
- [`spawningpool list`](#spawningpool-list) — list names
- [`spawningpool show`](#spawningpool-show) — print one definition
- [`spawningpool delete`](#spawningpool-delete) — remove an entity
- [`spawningpool run`](#spawningpool-run) — run a specialist, workflow, or tool
- [`spawningpool tui`](#spawningpool-tui) — browse and manage everything interactively

---

## `spawningpool` (bare)

State-aware onboarding. Reads the registry and prints exactly which command to
run next in the provider → model → specialist → run progression, plus a warning
for any provider whose API-key env var isn't set.

```sh
spawningpool
```

---

## `spawningpool define`

### provider

A wire protocol (`--api`) + endpoint (`--base-url`) + optional key env var.

```sh
spawningpool define provider <name> \
  --api <anthropic|openai> \
  --base-url <url> \
  [--api-key-env <ENV_VAR>] \
  [--constrained-decoding]
```

`--api` accepts the protocol or its brand alias:
- `anthropic-messages` (alias `anthropic`) — Claude.
- `openai-completions` (alias `openai`) — LM Studio / any OpenAI-compatible endpoint.

`--api-key-env` names the environment variable the key is read from *at run time*
(it stores the variable name, not the key). Omit it for keyless servers like LM Studio.

`--constrained-decoding` declares that this endpoint supports grammar-constrained
`response_format` (true constrained decoding). It's a capability you assert, not
something inferred — two `openai-completions` endpoints can differ. It only
affects how a constrained specialist forces its tool call; see
[`--constraint`](#specialist) below.

**Only `openai-completions` providers honor this flag.** `anthropic` providers
ignore it and always force the call with native tool choice, so setting it on an
Anthropic provider has no effect.

```sh
# Hosted Claude
spawningpool define provider anthropic --api anthropic \
  --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY

# Local LM Studio (keyless)
spawningpool define provider lmstudio --api openai --base-url http://localhost:1234/v1
```

### model

Keyed by its API `id`, defined under a provider, with its token limits. There is
**no built-in catalog** — you define the models you call. A model inherits its
`api`/`base-url` from its provider.

```sh
spawningpool define model <id> \
  --provider <provider> \
  --max-tokens <n> \
  --context-window <n> \
  [--name <display-name>]      # defaults to <id>
```

```sh
spawningpool define model claude-opus-4-8 --provider anthropic \
  --max-tokens 4096 --context-window 200000 --name "Claude Opus 4.8"
```

Defining a model whose `--provider` doesn't exist is rejected with the command
to create it.

### specialist

A system prompt on a (provider, model), with tools it may call.

```sh
spawningpool define specialist <name> \
  --provider <provider> \
  --model <model> \
  --system-prompt '<prompt>' \
  [--tools 'a,b,c']            # comma-separated; the model freely calls these
  [--constraint 'tool']        # OR: force exactly one call to this tool
  [--reasoning off|low|medium|high]   # default off
  [--stream]                   # stream output token-by-token (only visible with `run specialist --output plaintext`)
```

`--tools` and `--constraint` are **mutually exclusive**:

- `--tools` runs the specialist **agentically**: the model decides which tools to
  call, each backing script runs, its output is fed back, and the loop repeats
  until the model stops calling tools (cap: 16 turns).
- `--constraint` **forces** one call to the named tool — for a specialist whose
  sole job is to figure something out and hand it to a tool. The call runs once
  and the run ends. A forced call is incompatible with reasoning, so a
  constrained specialist must keep `--reasoning off` (enforced at define time).

  How the forced call is realized depends on the provider. By default it uses the
  portable **tool-call trick** — forcing a tool call (`tool_choice: "required"`
  on `openai-completions`, native forced tool choice on `anthropic`) whose
  arguments are the structured output — which works on every provider out of the
  box. True grammar-constrained decoding is an opt-in upgrade on top:
  - If an `openai-completions` provider was defined `--constrained-decoding`, the
    call instead uses grammar-constrained decoding (`response_format` built from
    the tool's parameter schema) — the model's output is guaranteed to match the
    schema. (Streaming is disabled for this run, since its single output is the
    tool arguments.)
  - Every other case — any `anthropic` provider, or an `openai-completions`
    provider without the flag — uses the tool-call trick above.

```sh
# Agentic: the model picks tools
spawningpool define specialist netop --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You diagnose network issues.' --tools 'ping,dig' --reasoning low

# Constrained: always classify via one tool
spawningpool define specialist classifier --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'Classify the message, then call classify.' --constraint classify

# Streaming, no tools
spawningpool define specialist writer --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You write concise release notes.' --stream
```

References to a missing provider, model, or tool are rejected with the exact
command to create the missing piece.

### tool

Defines a tool from an executable script. Its `# desc:` and `# params:` header
comments become the description and parameters the model sees. See
[Writing tools](tools.md) for the script format.

```sh
spawningpool define tool <name> --script <path>
```

```sh
spawningpool define tool ping --script ./scripts/ping.sh
```

A tool `<name>` is at most 64 characters of ASCII letters, digits, `_`, or `-`
(no dots or spaces), so it maps unambiguously to its script's file stem.

The script is checked at define time: it must exist and be executable
(`chmod +x`), failing here with a fix rather than mid-run. A missing `# desc:`
header warns but succeeds. Rather than recording the tool in `registry.json`,
this symlinks the script into the tools folder (`tools/` beside the registry),
which is the source of truth — so you can equally drop an executable script into
that folder by hand, edit one in place (its header is re-read on every run), or
`rm` one. Re-defining a tool replaces whatever backed that name.

---

## `spawningpool list`

Prints names, sorted, one per line.

```sh
spawningpool list providers
spawningpool list models
spawningpool list specialists
spawningpool list tools          # reads the tools folder, not the registry

# Special: query a running LM Studio server for the model ids it has loaded
# (at $LMSTUDIO_BASE_URL, default http://localhost:1234), not the registry.
spawningpool list models --remote
```

---

## `spawningpool show`

Prints one definition as pretty JSON. Errors if it doesn't exist.

```sh
spawningpool show provider anthropic
spawningpool show model claude-opus-4-8
spawningpool show specialist netop
spawningpool show tool ping
```

---

## `spawningpool delete`

Removes one entity, after asking for confirmation. Deleting a provider, model,
or tool that specialists still reference first warns about each reference the
delete would orphan, then prompts `delete <what>? [y/N]` — anything but `y`/`yes`
cancels. Pass `--yes` (`-y`) to skip the prompt (the orphan warnings still
print). Deleting a tool removes its script from the tools folder.

```sh
spawningpool delete specialist netop
spawningpool delete model claude-opus-4-8
spawningpool delete provider anthropic -y     # no prompt
spawningpool delete tool ping
```

---

## `spawningpool run`

Runs a specialist, a workflow, or a tool. Alias: `spawningpool spawn`.

```sh
spawningpool run specialist <name> --prompt '<prompt>' [--output <json|plaintext>]
spawningpool run workflow <name> [--arg KEY=VALUE]...
spawningpool run tool <name> [--arg KEY=VALUE]...
```

### `run specialist`

Instantiates a specialist with a prompt and runs it.

```sh
spawningpool run specialist netop --prompt 'Why can I not reach example.com?'
```

`--output` chooses the format:

- **`json`** (the default) — a single machine-readable JSON object on stdout
  once the run finishes, with fields: `output` (assistant text), `thinking`,
  `inputTokens`, `outputTokens`, `stopReason`, `model`, `specialist`, `turns`,
  and `toolCalls` (each `{name, success, output}`). Nothing is streamed and
  nothing goes to stderr — even a specialist defined `--stream` is buffered into
  `output`.
- **`plaintext`** — terminal-friendly streaming:
  - **stdout** — assistant text (streamed live if the specialist was defined
    `--stream`) and each tool's output (`[tool <name>]`).
  - **stderr** — token usage (`[usage] N in / N out`) and tool failures.

```sh
# Pipe-friendly default: parse the JSON envelope
spawningpool run specialist netop --prompt 'reachable?' | jq -r .output

# Watch it stream in a terminal
spawningpool run specialist writer --prompt 'tagline' --output plaintext
```

The API key is sourced from the provider's `--api-key-env` variable at run time;
if it isn't set you'll get an auth error (bare `spawningpool` warns about this in advance).
An agentic specialist that never stops calling tools fails after 16 turns.

### `run workflow`

Executes a workflow from the `workflows/` folder beside the registry, by name
(the file name with any extension stripped, like tools). The workflow is parsed,
type-checked against the tool catalog and registry, then evaluated; its result
value (the last statement's) is printed as JSON on stdout. See
[the Workflow DSL](workflow-dsl.md).

```sh
spawningpool run workflow triage --arg CITY=Portland --arg COUNT=3
```

Each `--arg KEY=VALUE` supplies one of the workflow's declared `# inputs:`
([§5.1](workflow-dsl.md#51-inputs)), repeatable. The value is coerced to the
input's declared type: a `string` takes the text verbatim, `number`/`bool` parse
the scalar, and an array/object input parses its value as JSON. Every declared
input must be supplied, and an `--arg` that names no declared input is rejected —
both fail before the workflow runs.

If the run is invoked with `$SP_OUTPUT_PATH` set, the result JSON is also written
there (in addition to stdout), matching the contract a tool obeys — so a workflow
is composable as a tool by an outer runner.

A workflow can also nest other workflows in-language with the `run` verb
([§6.8](workflow-dsl.md#68-workflow-call)); the whole reachable closure is loaded,
type-checked (callee result types inferred recursively), and cycle-checked before
anything runs. Because `run` resolves in `workflows/` and `call` in `tools/`, a
tool and a workflow may share a name without ambiguity.

Each specialist invoked by the workflow's `ask` expressions authenticates with
its own provider's key, sourced from that provider's `--api-key-env`; a workflow
can freely mix providers. A workflow that only calls tools needs no key.

### `run tool`

Runs a single tool script directly, passing parameters as `KEY=VALUE`. Prints
the JSON the tool writes to `$SP_OUTPUT_PATH`.

```sh
spawningpool run tool ping --arg HOST=example.com --arg COUNT=3
```

---

## `spawningpool tui`

Opens an interactive terminal UI over the same registry and tools folder,
backing every `define`/`list`/`show`/`delete` with keyboard navigation.

```sh
spawningpool tui
```

The footer always shows the keys for the current mode; press `?` for the full
list. The essentials, in normal mode:

| Key | Action |
| --- | --- |
| `p` / `s` / `t` | switch the providers / specialists / tools tabs |
| `h` `j` `k` `l` | navigate (drill in/out, move the selection) |
| `a` | add an entity |
| `o` | open (drill into a provider's models) |
| `e` | edit the selected entity's JSON in `$EDITOR` |
| `r` | rename |
| `d` | delete (confirmed with `y`/`n`) |
| `/` | filter the list |
| `^r` | reload from disk |
| `q` | quit |
