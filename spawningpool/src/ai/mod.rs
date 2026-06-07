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
//! ## Tool-call validation (opt-in)
//!
//! Tools here are built *dynamically*: [`Tool::parameters`] is a JSON Schema
//! ([`serde_json::Value`]) assembled at runtime, not derived from a compile-time
//! Rust type — so `schemars` / `#[derive(JsonSchema)]` deliberately is **not**
//! used (there is no static type to derive from, and forcing one would fight the
//! dynamic-agent design).
//!
//! Because the schema is dynamic there is no compile-time safety net, so
//! [`validate_tool_call`] is the only check available. It is opt-in: the default
//! path passes a model's tool call through unvalidated. A caller that wants
//! strictness validates and, on failure, feeds an error result back to the model
//! to retry:
//!
//! ```no_run
//! # use spawningpool::ai::{validate_tool_call, ContentBlock, Tool};
//! # fn f(tool: &Tool, call: &ContentBlock) -> Option<ContentBlock> {
//! if let ContentBlock::ToolCall { id, .. } = call {
//!     if let Err(e) = validate_tool_call(tool, call) {
//!         // Hand the violations back to the model so it can fix the call.
//!         return Some(ContentBlock::tool_error(id, e.to_string()));
//!     }
//! }
//! # None
//! # }
//! ```
//!
//! The adapters' best-effort `parse_args` fallback (malformed tool-call JSON
//! becomes a `Value::String`) stays as the transport-level behavior; argument
//! *shape* validation is this separate, caller-driven concern layered on top.

pub mod catalog;
pub mod client;
pub mod message;
pub mod model;
pub mod provider;
pub(crate) mod providers;
pub(crate) mod sse;
pub mod validation;

pub use catalog::{get_model, get_models, get_providers};
pub use client::Client;
pub use message::{ContentBlock, Message, Role, StopReason, Usage};
pub use model::{Api, Context, Model, Tool};
pub use provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, ProviderRegistry, Reasoning,
    StreamEvent,
};
pub use validation::{validate_tool_call, ToolValidationError};
