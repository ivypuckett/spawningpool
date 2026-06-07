//! Multi-provider LLM abstraction.
//!
//! One normalized domain model ([`Message`], [`ContentBlock`], [`Usage`]) sits
//! in the middle; thin per-provider adapters translate it to and from each
//! wire format at the edges. Models are data ([`Model`]) looked up from a
//! [`catalog`], and the provider is chosen at runtime from `model.api` via a
//! [`ProviderRegistry`]. Ships with Claude (`anthropic-messages`) and LM Studio
//! (`openai-completions`) adapters.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use spawningpool::ai::{get_model, Client, Context, Message, CompleteOptions};
//!
//! let client = Client::new();
//! // Pick the provider at runtime — swap "anthropic"/"lmstudio" freely.
//! let model = get_model("anthropic", "claude-opus-4-8")?;
//! let ctx = Context::new(None, vec![Message::user("Say hi")]);
//! let reply = client.complete(&model, &ctx, &CompleteOptions::default()).await?;
//! println!("{:?}", reply.message.content);
//! # Ok(())
//! # }
//! ```
//!
//! ## FUTURE_AGENT: typed tool-parameter validation
//!
//! Tool parameters are currently an untyped [`serde_json::Value`] JSON Schema
//! ([`Tool::parameters`]), and tool-call `arguments` returned by a model are
//! passed through unvalidated. A future task should add a typed validation
//! layer so callers define tool inputs as Rust types and tool calls are
//! checked against the schema before reaching application code. Concretely:
//!
//! - Add a `schemars` dependency and let `Tool` be constructed from a type
//!   implementing `JsonSchema` (e.g. `Tool::typed::<MyArgs>(name, desc)`),
//!   deriving `parameters` from the schema instead of hand-written JSON.
//! - Add a `validate_tool_call(tools, &ContentBlock::ToolCall { .. })` helper
//!   (using the `jsonschema` crate) that returns a structured error when a
//!   model's `arguments` don't match the declared schema, so the caller can
//!   feed an error `ToolResult` back to the model and let it retry — mirroring
//!   how pi-ai surfaces `validateToolCall` failures.
//! - Keep the untyped `Tool::parameters` path working; typed construction is
//!   additive sugar over it, not a replacement.
//! - When parsing tool-call arguments in the adapters, surface malformed JSON
//!   as a typed error rather than the current best-effort
//!   `Value::String` fallback (see `parse_args` in the adapter modules).

pub mod catalog;
pub mod client;
pub mod message;
pub mod model;
pub mod provider;
pub(crate) mod providers;
pub(crate) mod sse;

pub use catalog::{get_model, get_models, get_providers};
pub use client::Client;
pub use message::{ContentBlock, Cost, Message, Role, StopReason, Usage};
pub use model::{Api, Context, CostRates, Model, Tool};
pub use provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, ProviderRegistry, Reasoning,
    StreamEvent,
};
