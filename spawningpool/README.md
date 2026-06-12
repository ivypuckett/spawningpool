# spawningpool

[![crates.io](https://img.shields.io/crates/v/spawningpool.svg)](https://crates.io/crates/spawningpool)
[![docs.rs](https://img.shields.io/docsrs/spawningpool)](https://docs.rs/spawningpool)
[![license](https://img.shields.io/crates/l/spawningpool.svg)](https://github.com/ivypuckett/spawningpool/blob/main/LICENSE)

Create hyper-specific, 0-waste agents.

`spawningpool` is the core library behind the [`spawningpool`](https://crates.io/crates/spawningpool-cli)
CLI. A **specialist** is a saved template of `(provider, model, system prompt,
tools)` that you instantiate with a prompt and run. This crate gives you the
domain types, the on-disk registry, the provider clients (Anthropic and any
OpenAI-compatible endpoint), and the agentic run loop that drives a specialist
against a prompt — executing its tools and feeding results back until it settles.

If you just want the command-line tool, install
[`spawningpool-cli`](https://crates.io/crates/spawningpool-cli) instead.

## Install

```sh
cargo add spawningpool
```

## What's in the box

- **`domain`** — the entity model: [`Registry`], [`ProviderDef`], [`ModelDef`],
  [`Specialist`], and [`ToolDef`], with reference resolution between them.
- **`store`** — load and save the JSON registry on disk.
- **`tools`** — resolve executable scripts in the `tools/` folder into [`ToolDef`]s.
- **`ai`** — the [`Client`], provider adapters, message/streaming types, and
  optional JSON-Schema validation of tool calls.
- **`run_specialist`** — the front-end-agnostic agentic loop, which reports
  progress through a [`RunEvent`] observer you supply rather than writing to
  stdout itself.

## Example

Load the registry the `spawningpool` CLI persists, then run a specialist and print its
output as it streams:

```rust,no_run
use spawningpool::ai::Client;
use spawningpool::{run_specialist, store, tools, RunEvent};

#[tokio::main]
async fn main() -> Result<(), String> {
    // Load the registry the `spawningpool` CLI writes to disk.
    let registry = store::load()?;
    let specialist = registry
        .specialists
        .get("haiku-namer")
        .ok_or("unknown specialist")?;

    // Resolve the specialist's tools from the tools/ folder beside the registry.
    let resolved = tools::resolve_all(&store::tools_dir(), specialist.tool_names())?;

    // Build request options, sourcing the API key from the provider's env var.
    let mut opts = specialist.complete_options();
    if let Some(provider) = registry.providers.get(&specialist.provider) {
        if let Some(env) = provider.api_key_env.as_ref() {
            if let Ok(key) = std::env::var(env) {
                opts.api_key = Some(key);
            }
        }
        opts.constrained_decoding = provider.constrained_decoding;
    }

    // Drive the agentic loop, printing assistant text as it arrives.
    let client = Client::new();
    let mut observer = |event: RunEvent<'_>| {
        if let RunEvent::Text(text) | RunEvent::TextDelta(text) = event {
            print!("{text}");
        }
    };
    run_specialist(
        &client,
        &registry,
        specialist,
        "A CLI that spawns AI specialists",
        &resolved,
        &opts,
        &mut observer,
    )
    .await
}
```

## Forcing structured output

A [`Specialist`] with a `constraint` forces one call to a named tool, making the
tool's parameters a structured-output schema. By default this uses the
**tool-call trick**: rather than relying on grammar-constrained decoding (which
not every endpoint supports), the request forces a tool call whose arguments are
the structured output — `tool_choice` on OpenAI-compatible endpoints, native
forced tool choice on Anthropic — so it works on every provider.

Setting `constrained_decoding` (from a [`ProviderDef`] declared with
`--constrained-decoding`) upgrades to true grammar-constrained decoding. Only the
OpenAI-compatible adapter honors it; the Anthropic adapter always uses native
forced tool choice.

## Documentation

- API docs: <https://docs.rs/spawningpool>
- Guides (quickstart, CLI reference, writing tools, configuration):
  [the `docs/` folder](https://github.com/ivypuckett/spawningpool/tree/main/docs)

## License

Licensed under the [MIT license](https://github.com/ivypuckett/spawningpool/blob/main/LICENSE).

[`Registry`]: https://docs.rs/spawningpool/latest/spawningpool/struct.Registry.html
[`ProviderDef`]: https://docs.rs/spawningpool/latest/spawningpool/struct.ProviderDef.html
[`ModelDef`]: https://docs.rs/spawningpool/latest/spawningpool/struct.ModelDef.html
[`Specialist`]: https://docs.rs/spawningpool/latest/spawningpool/struct.Specialist.html
[`ToolDef`]: https://docs.rs/spawningpool/latest/spawningpool/struct.ToolDef.html
[`Client`]: https://docs.rs/spawningpool/latest/spawningpool/ai/struct.Client.html
[`RunEvent`]: https://docs.rs/spawningpool/latest/spawningpool/enum.RunEvent.html
