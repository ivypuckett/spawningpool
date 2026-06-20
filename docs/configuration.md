# Configuration

Everything is configured through environment variables and the on-disk registry.
There is no config file to edit.

## Registry location

All definitions persist to a single JSON file. Its path is resolved in this
order (first match wins):

| Variable | Effect |
| --- | --- |
| `SPAWNINGPOOL_REGISTRY` | Exact path to the registry file. Wins over everything. |
| `SPAWNINGPOOL_HOME` | Directory holding `registry.json`. |
| `HOME` | Falls back to `~/.spawningpool/registry.json` (the default). |
| *(none set)* | Relative `.spawningpool/registry.json`. |

A missing file loads as an empty registry, so the first `spawningpool define` creates it.
Writes are atomic (temp file + rename), so a crash can't leave it half-written.

```sh
# Inspect the default registry
cat ~/.spawningpool/registry.json

# Use an isolated registry for an experiment
SPAWNINGPOOL_REGISTRY=/tmp/scratch.json spawningpool define provider test --api openai \
  --base-url http://localhost:1234/v1
SPAWNINGPOOL_REGISTRY=/tmp/scratch.json spawningpool list providers
```

## API keys

A provider stores the *name* of the env var its key comes from (`--api-key-env`),
not the key itself. The key is read from that variable at run time.

```sh
spawningpool define provider anthropic --api anthropic \
  --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY
export ANTHROPIC_API_KEY=sk-ant-...
```

Run bare `spawningpool` to be warned about any provider whose key variable isn't set
before a run hits the error.

### Built-in fallbacks

If a provider has no `--api-key-env` (or its variable is unset), the adapters
still check these directly:

| Provider API | Fallback variable | Notes |
| --- | --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` | Required; a run errors without a key. |
| `openai` | `LMSTUDIO_API_KEY` | Optional — LM Studio is keyless; only sent if set. |

## Forcing structured output (the tool-call trick)

A specialist defined with `--constraint <tool>` is forced to produce one call to
that tool, so the tool's parameters become a structured-output schema. By default
this is realized with the **tool-call trick**: rather than relying on
grammar-constrained decoding (which not every endpoint supports), the request
forces a tool call whose arguments are the structured output. This works on every
provider out of the box — `tool_choice: "required"` on OpenAI-compatible
endpoints, native forced tool choice on Anthropic.

Grammar-constrained decoding (a hard, token-level guarantee) is an opt-in upgrade,
declared per provider with `--constrained-decoding`:

| Provider API | `--constrained-decoding` |
| --- | --- |
| `openai-completions` | Honored — forces the call via a strict `response_format` JSON schema. |
| `anthropic-messages` | Ignored — always uses native forced tool choice. |

It's a capability you assert, not something inferred: two `openai-completions`
endpoints can differ, so the flag declares what the endpoint behind this provider
supports. See [CLI reference → define provider](cli.md#provider) and
[Writing tools → forcing a tool call](tools.md#forcing-a-tool-call-constraint).

## LM Studio

| Variable | Default | Used by |
| --- | --- | --- |
| `LMSTUDIO_BASE_URL` | `http://localhost:1234` | `spawningpool list models --remote` discovery. |
| `LMSTUDIO_API_KEY` | *(unset)* | Optional bearer token for OpenAI-compatible requests. |

```sh
# Point discovery at a non-default LM Studio host
LMSTUDIO_BASE_URL=http://192.168.1.50:1234 spawningpool list models --remote
```

## Apple Foundation Models (macOS 27+)

macOS 27 ships the `fm` CLI, which exposes Apple's on-device Foundation Model
through an OpenAI-compatible server — so spawningpool talks to it with the stock
`openai-completions` adapter, no Apple-specific code. Start the server (it
defaults to `127.0.0.1:1976`), then point a provider at it:

```sh
fm serve                       # leave running; serves /v1/chat/completions

spawningpool define provider apple --api openai \
  --base-url http://127.0.0.1:1976 --constrained-decoding
spawningpool define model system --provider apple \
  --name "Apple Foundation (on-device)" --max-tokens 4096 --context-window 8192
```

The two model ids `fm serve` exposes are `system` (on-device, always available)
and `pcc` (Apple Foundation Model on Private Cloud Compute). It needs no API key.
`--constrained-decoding` is safe to set — `fm serve` honors strict
`response_format` JSON schemas — and streaming, tool calls, and `reasoning`
all work through the existing adapter.

> The on-device `system` model is small: forced/constrained tool calls
> (`--constraint`) are reliable, but *agentic* auto tool-calling is hit-or-miss
> (the model sometimes returns empty rather than calling). Prefer a constraint
> when you need a guaranteed structured call.

## Building from source

```sh
cargo build              # debug build → target/debug/spawningpool
cargo build --release    # optimized → target/release/spawningpool
cargo test               # run the test suite
cargo clippy             # lint
cargo fmt                # format
```

The compiled binary is `target/release/spawningpool` (or `target/debug/spawningpool`).
Put it on your `PATH` to invoke it as `spawningpool`, the way the docs show:

```sh
export PATH="$PWD/target/release:$PATH"
```
