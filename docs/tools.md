# Writing tools

A tool is a single **executable script** that lives in a folder. There's no YAML,
no task runner, no plugin API, and no registry entry — you add a tool by putting a
script in the folder, and the runner executes it when the model calls it.

## Where tools live

Tools live in an `ls`-able folder: `tools/` next to your registry, so
`~/.spawningpool/tools/` by default. That folder **is** the catalog — there's
nothing about tools in `registry.json`, so you manage them the way you'd manage
any folder of scripts.

- **A tool's name is its file name**, with any extension stripped — a script (or
  symlink) named `ping` and a file `ping.sh` both become the `ping` tool.
- **The header is read on every run**, so editing a script changes its description
  and parameters live; there's nothing to re-sync or re-define.
- **Add one** either by dropping an executable script into the folder yourself:
  ```sh
  install -D -m 755 ./ping.sh ~/.spawningpool/tools/ping
  ```
  or by letting `spawningpool define tool` symlink one in for you — handy when the script
  lives in a project repo and you want it tracked there, not copied:
  ```sh
  spawningpool define tool ping --script ./ping.sh
  ```
- **See what's there** with `spawningpool list tools`, inspect one with `spawningpool show tool <name>`,
  and remove one with `spawningpool delete tool <name>` (or just `rm` the file).

## The script format

```sh
#!/bin/sh
# desc: Greet a person by name
# params: NAME, GREETING
echo "$GREETING, $NAME!"
```

- `# desc:` → the tool **description** the model reads. Missing is allowed (you'll
  get a warning and an empty description), but a good description is what makes
  the model call the tool correctly.
- `# params:` → the tool's **parameters**, separated by whitespace and/or commas.
  Each becomes a required string argument.
- For each directive, the **first** matching comment line wins. Non-comment lines
  are ignored, so the header can sit anywhere.

Each argument the model supplies is passed to the script as an **environment
variable of the same name** — never interpolated into a command line, so there's
no shell-injection surface. Non-string JSON values are passed as their JSON text
(e.g. `3`, `true`, `[1,2]`).

## Requirements

- A shebang line (e.g. `#!/bin/sh`, `#!/usr/bin/env python3`).
- The executable bit set: `chmod +x your-script.sh`.

`spawningpool define tool` checks both up front, so a broken script fails immediately with
a fix. A script you drop into the folder by hand is checked when it's first run.

## Exit codes

- **Exit 0** → success; combined stdout+stderr is returned to the model as the
  tool result.
- **Non-zero exit** → reported to the model as a tool *error* (agentic
  specialists can read it and retry) or surfaced (constrained specialists).

## End-to-end example

```sh
# 1. Write the script
cat > ping.sh <<'EOF'
#!/bin/sh
# desc: Ping a host a few times and report whether it responds
# params: HOST
ping -c 3 "$HOST"
EOF
chmod +x ping.sh

# 2. Make it a tool: symlink it into the tools folder.
#    (Or drop it in directly: install -D -m 755 ./ping.sh ~/.spawningpool/tools/ping)
spawningpool define tool ping --script ./ping.sh

# 3. Confirm it's there and what the model will see (header is read live)
spawningpool list tools
spawningpool show tool ping

# 4. Give it to a specialist (agentic — the model decides when to call it)
spawningpool define specialist netop --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You diagnose reachability problems using the tools available.' \
  --tools ping

# 5. Run
spawningpool run --specialist netop --prompt 'Can you reach example.com?'
```

## Forcing a tool call (constraint)

When a specialist's only job is to compute something and hand it off, force the
call with `--constraint` instead of `--tools`. The model is guaranteed to call
that one tool exactly once, then the run ends.

```sh
cat > classify.sh <<'EOF'
#!/bin/sh
# desc: Record a sentiment classification
# params: LABEL
echo "recorded: $LABEL"
EOF
chmod +x classify.sh
spawningpool define tool classify --script ./classify.sh

spawningpool define specialist sentiment --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'Classify the sentiment as positive, negative, or neutral, then call classify.' \
  --constraint classify          # note: reasoning must stay off with --constraint

spawningpool run --specialist sentiment --prompt 'I absolutely love this!'
```

The forced call works on any provider out of the box via the **tool-call trick**:
rather than relying on grammar-constrained decoding (which not every endpoint
supports), it forces a tool call whose arguments are the structured output
(`tool_choice: "required"` on OpenAI-compatible endpoints, native forced tool
choice on Anthropic). If an OpenAI-compatible endpoint supports grammar-constrained
output — many local servers like LM Studio do — define that provider with
`--constrained-decoding` to upgrade to a hard, token-level guarantee that the
arguments match the tool's schema (`anthropic` providers ignore the flag):

```sh
spawningpool define provider lmstudio --api openai --base-url http://localhost:1234/v1 \
  --constrained-decoding
```

See [CLI reference → define provider](cli.md#provider) for the details.

## Chaining specialists

There is no built-in chain abstraction — by design. A specialist's stdout is
plain text, so compose them with the shell:

```sh
topic=$(spawningpool run --specialist haiku-namer --prompt 'a tool that spawns agents')
spawningpool run --specialist writer --prompt "Write a one-line tagline for: $topic"
```

For richer orchestration, drive `spawningpool run` from any language or a workflow tool
(e.g. LangGraph, Mastra).
