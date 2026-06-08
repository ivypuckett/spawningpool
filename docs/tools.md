# Writing tools

A tool is a single **executable script**. There is no YAML, no task runner, and
no plugin API — `sp define tool` reads two header comments to build the tool the
model sees, and the runner executes the script when the model calls it.

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

Both are checked at `sp define tool` time, so a broken script fails immediately
with a fix instead of mid-run.

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

# 2. Register it as a tool
sp define tool ping --script ./ping.sh

# 3. Confirm what the model will see
sp show tool ping

# 4. Give it to a specialist (agentic — the model decides when to call it)
sp define specialist netop --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You diagnose reachability problems using the tools available.' \
  --tools ping

# 5. Run
sp run --specialist netop --prompt 'Can you reach example.com?'
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
sp define tool classify --script ./classify.sh

sp define specialist sentiment --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'Classify the sentiment as positive, negative, or neutral, then call classify.' \
  --constraint classify          # note: reasoning must stay off with --constraint

sp run --specialist sentiment --prompt 'I absolutely love this!'
```

## Chaining specialists

There is no built-in chain abstraction — by design. A specialist's stdout is
plain text, so compose them with the shell:

```sh
topic=$(sp run --specialist haiku-namer --prompt 'a tool that spawns agents')
sp run --specialist writer --prompt "Write a one-line tagline for: $topic"
```

For richer orchestration, drive `sp run` from any language or a workflow tool
(e.g. LangGraph, Mastra).
