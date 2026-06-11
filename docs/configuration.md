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

## LM Studio

| Variable | Default | Used by |
| --- | --- | --- |
| `LMSTUDIO_BASE_URL` | `http://localhost:1234` | `spawningpool list models --remote` discovery. |
| `LMSTUDIO_API_KEY` | *(unset)* | Optional bearer token for OpenAI-compatible requests. |

```sh
# Point discovery at a non-default LM Studio host
LMSTUDIO_BASE_URL=http://192.168.1.50:1234 spawningpool list models --remote
```

## Building from source

```sh
cargo build              # debug build → target/debug/spawningpool
cargo build --release    # optimized → target/release/spawningpool
cargo test               # run the test suite
cargo clippy             # lint
cargo fmt                # format
```

The binary is named `spawningpool`; alias it to `spawningpool` for the documented usage:

```sh
alias sp="$PWD/target/release/spawningpool"
```
