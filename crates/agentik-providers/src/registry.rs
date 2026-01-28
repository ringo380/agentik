//! Provider registry for managing available providers.

use std::collections::HashMap;
use std::sync::Arc;

use agentik_core::Config;

use super::anthropic::AnthropicProvider;
use super::local::LocalProvider;
use super::openai::OpenAIProvider;
use super::traits::{ModelInfo, Provider};

/// Registry of available AI providers.
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
    default_provider: Option<String>,
}

impl ProviderRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: None,
        }
    }

    /// Initialize registry with all available providers based on configuration.
    ///
    /// This method first tries to use API keys from the config, then falls back
    /// to environment variables (ANTHROPIC_API_KEY, OPENAI_API_KEY).
    pub fn from_config(config: &Config) -> Self {
        let mut registry = Self::new();

        // Register Anthropic provider if API key is available (config or env)
        let anthropic_key = config
            .providers
            .anthropic
            .as_ref()
            .and_then(|c| c.resolve_api_key())
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());

        if let Some(api_key) = anthropic_key {
            let provider = AnthropicProvider::new(&api_key);
            registry.register(Arc::new(provider));
        }

        // Register OpenAI provider if API key is available (config or env)
        let openai_key = config
            .providers
            .openai
            .as_ref()
            .and_then(|c| c.resolve_api_key())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok());

        if let Some(api_key) = openai_key {
            let mut provider = OpenAIProvider::new(&api_key);
            if let Some(ref openai_config) = config.providers.openai {
                if let Some(ref base_url) = openai_config.base_url {
                    provider = provider.with_base_url(base_url);
                }
            }
            // Also check OPENAI_BASE_URL env var
            if let Ok(base_url) = std::env::var("OPENAI_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            registry.register(Arc::new(provider));
        }

        // Register Local (Ollama) provider - always available (no API key needed)
        let local_enabled = config
            .providers
            .local
            .as_ref()
            .map(|c| c.enabled)
            .unwrap_or(true);

        if local_enabled {
            let provider = config
                .providers
                .local
                .as_ref()
                .and_then(|c| c.base_url.as_ref())
                .map(LocalProvider::with_url)
                .or_else(|| {
                    std::env::var("OLLAMA_HOST")
                        .ok()
                        .map(LocalProvider::with_url)
                })
                .unwrap_or_default();
            registry.register(Arc::new(provider));
        }

        // Set default provider based on config or first available
        if let Some(ref default) = config.providers.default_provider {
            registry.set_default(default);
        }

        registry
    }

    /// Initialize registry with providers from environment variables.
    pub fn from_env() -> Self {
        let mut registry = Self::new();

        // Check for Anthropic API key
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            registry.register(Arc::new(AnthropicProvider::new(&api_key)));
        }

        // Check for OpenAI API key
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let mut provider = OpenAIProvider::new(&api_key);
            // Support OpenAI-compatible base URLs
            if let Ok(base_url) = std::env::var("OPENAI_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            registry.register(Arc::new(provider));
        }

        // Always register local provider
        let local_url = std::env::var("OLLAMA_HOST")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        registry.register(Arc::new(LocalProvider::with_url(local_url)));

        registry
    }

    /// Register a provider.
    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        let id = provider.id().to_string();
        if self.default_provider.is_none() {
            self.default_provider = Some(id.clone());
        }
        self.providers.insert(id, provider);
    }

    /// Get a provider by ID.
    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }

    /// Get the default provider.
    pub fn default_provider(&self) -> Option<Arc<dyn Provider>> {
        self.default_provider.as_ref().and_then(|id| self.get(id))
    }

    /// Set the default provider.
    pub fn set_default(&mut self, id: &str) -> bool {
        if self.providers.contains_key(id) {
            self.default_provider = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// List all registered providers.
    pub fn list(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }

    /// Iterate over all registered providers.
    pub fn providers(&self) -> impl Iterator<Item = &Arc<dyn Provider>> {
        self.providers.values()
    }

    /// Get all available models across all providers.
    pub fn all_models(&self) -> Vec<ModelInfo> {
        self.providers
            .values()
            .flat_map(|p| p.available_models())
            .collect()
    }

    /// Find a model by ID across all providers.
    pub fn find_model(&self, model_id: &str) -> Option<(Arc<dyn Provider>, ModelInfo)> {
        for provider in self.providers.values() {
            if let Some(model) = provider
                .available_models()
                .into_iter()
                .find(|m| m.id == model_id)
            {
                return Some((provider.clone(), model));
            }
        }
        None
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
