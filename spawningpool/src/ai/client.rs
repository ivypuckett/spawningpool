//! The [`Client`]: the entry point that ties a [`ProviderRegistry`] to a shared
//! HTTP client and dispatches requests by the model's [`Api`].

use crate::ai::model::{Api, Context, Model};
use crate::ai::provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, ProviderRegistry,
};

/// Drives requests against whichever provider a model names.
///
/// The provider is selected at runtime from `model.api` via the registry, so
/// the same `Client` serves Claude and LM Studio (and anything else
/// registered) without recompilation.
#[derive(Clone)]
pub struct Client {
    registry: ProviderRegistry,
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

impl Client {
    /// A client with the built-in adapters registered.
    pub fn new() -> Self {
        Client {
            registry: ProviderRegistry::with_builtins(),
            http: reqwest::Client::new(),
        }
    }

    /// A client over a caller-supplied registry (e.g. to add or replace
    /// adapters at runtime).
    pub fn with_registry(registry: ProviderRegistry) -> Self {
        Client {
            registry,
            http: reqwest::Client::new(),
        }
    }

    /// Mutable access to the registry, for registering adapters at runtime.
    pub fn registry_mut(&mut self) -> &mut ProviderRegistry {
        &mut self.registry
    }

    fn provider_for(&self, api: Api) -> Result<std::sync::Arc<dyn Provider>, Error> {
        self.registry
            .get(api)
            .ok_or_else(|| Error::Config(format!("no provider registered for api: {api:?}")))
    }

    /// Send a request and await the full response.
    pub async fn complete(
        &self,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<Completion, Error> {
        self.provider_for(model.api)?
            .complete(&self.http, model, ctx, opts)
            .await
    }

    /// Send a request and return a stream of normalized events.
    pub async fn stream(
        &self,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<EventStream, Error> {
        self.provider_for(model.api)?
            .stream(&self.http, model, ctx, opts)
            .await
    }

    /// Discover the models a running LM Studio instance currently has loaded,
    /// via `GET {base_url}/v1/models`.
    pub async fn list_models(&self, provider: &str) -> Result<Vec<Model>, Error> {
        match provider {
            "anthropic" => Ok(crate::ai::catalog::get_models("anthropic")),
            "lmstudio" => crate::ai::providers::openai::list_models(&self.http).await,
            other => Err(Error::Config(format!("unknown provider: {other}"))),
        }
    }
}
