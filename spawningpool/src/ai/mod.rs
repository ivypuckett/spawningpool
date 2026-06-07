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
//! ## FUTURE_AGENT: optional runtime tool-call validation
//!
//! Tools in this project are built *dynamically*: [`Tool::parameters`] is a
//! JSON Schema ([`serde_json::Value`]) assembled at runtime, not derived from a
//! compile-time Rust type. So the usual `schemars` / `#[derive(JsonSchema)]`
//! approach does **not** fit here — there is no static type to derive from, and
//! forcing one would fight the dynamic-agent design. Do not add it.
//!
//! What *does* fit is optional, runtime validation of a model's tool-call
//! `arguments` against the tool's runtime schema. Because the schema is dynamic
//! there is no compile-time safety net, so a runtime validator is the only
//! check available — but it should stay opt-in, not forced:
//!
//! - Add a `validate_tool_call(tool: &Tool, call: &ContentBlock)` helper (e.g.
//!   via the `jsonschema` crate) that checks a `ToolCall`'s `arguments` `Value`
//!   against the `Tool::parameters` `Value` and returns a structured error.
//! - Leave the default path unvalidated. A caller that wants strictness calls
//!   the validator and, on failure, feeds an error `ToolResult` back to the
//!   model so it can retry — mirroring pi-ai's `validateToolCall`. A caller
//!   that wants raw pass-through simply never calls it.
//! - Keep the adapters' best-effort `parse_args` fallback (malformed tool-call
//!   JSON becomes a `Value::String`) as the transport-level behavior; argument
//!   *shape* validation is a separate, caller-driven concern layered on top.

pub mod catalog;
pub mod client;
pub mod message;
pub mod model;
pub mod provider;
pub(crate) mod providers;
pub(crate) mod sse;

pub use catalog::{get_model, get_models, get_providers};
pub use client::Client;
pub use message::{ContentBlock, Message, Role, StopReason, Usage};
pub use model::{Api, Context, Model, Tool};
pub use provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, ProviderRegistry, Reasoning,
    StreamEvent,
};
