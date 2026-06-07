//! Built-in provider adapters.
//!
//! Each adapter translates the unified message types to and from one wire
//! protocol. Adding a provider means adding a module here, an [`crate::ai::Api`]
//! variant, and registering it in
//! [`crate::ai::ProviderRegistry::with_builtins`] — the core does not change.

pub(crate) mod anthropic;
pub(crate) mod openai;
