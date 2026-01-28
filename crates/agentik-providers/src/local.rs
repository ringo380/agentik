//! Local model provider (Ollama) implementation.
//!
//! Ollama exposes an OpenAI-compatible API, so this provider
//! wraps the OpenAI provider with Ollama-specific defaults.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use agentik_core::{Message, ToolCall, ToolDefinition, ToolResult};

use crate::traits::{
    CompletionRequest, CompletionResponse, ModelInfo, Provider, StreamChunk, ToolCapable,
};
use crate::openai::OpenAIProvider;

/// Default Ollama API URL.
const OLLAMA_API_URL: &str = "http://localhost:11434/v1";

/// Local provider for Ollama models.
pub struct LocalProvider {
    /// Inner OpenAI-compatible provider
    inner: OpenAIProvider,
    /// Ollama-specific client for model listing
    client: Client,
    /// Base URL for Ollama API
    base_url: String,
}

impl LocalProvider {
    /// Create a new local provider connecting to Ollama.
    pub fn new() -> Self {
        Self::with_url(OLLAMA_API_URL)
    }

    /// Create with a custom Ollama URL.
    pub fn with_url(url: impl Into<String>) -> Self {
        let url = url.into();
        // Ollama doesn't require an API key, but OpenAI provider expects one
        let inner = OpenAIProvider::new("ollama")
            .with_base_url(&url)
            .with_default_model("llama3.2");

        Self {
            inner,
            client: Client::new(),
            base_url: url,
        }
    }

    /// Set the default model.
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.inner = OpenAIProvider::new("ollama")
            .with_base_url(&self.base_url)
            .with_default_model(model);
        self
    }

    /// Check if Ollama is running.
    pub async fn is_running(&self) -> bool {
        // Try to list models to check if Ollama is available
        let base = self.base_url.trim_end_matches("/v1");
        match self.client.get(format!("{}/api/tags", base)).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// List available models from Ollama.
    pub async fn list_models(&self) -> anyhow::Result<Vec<OllamaModel>> {
        let base = self.base_url.trim_end_matches("/v1");
        let response = self
            .client
            .get(format!("{}/api/tags", base))
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to list Ollama models");
        }

        let tags: OllamaTags = response.json().await?;
        Ok(tags.models)
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for LocalProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn name(&self) -> &str {
        "Local (Ollama)"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        // Return some common models that might be available
        // In practice, you'd want to call list_models() to get actual models
        vec![
            ModelInfo {
                id: "llama3.2".to_string(),
                name: "Llama 3.2".to_string(),
                provider: "local".to_string(),
                context_window: 128_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None, // Local = free
            },
            ModelInfo {
                id: "llama3.2:1b".to_string(),
                name: "Llama 3.2 1B".to_string(),
                provider: "local".to_string(),
                context_window: 128_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
            ModelInfo {
                id: "codellama".to_string(),
                name: "Code Llama".to_string(),
                provider: "local".to_string(),
                context_window: 16_000,
                max_output_tokens: 4_096,
                supports_tools: false,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
            ModelInfo {
                id: "mistral".to_string(),
                name: "Mistral".to_string(),
                provider: "local".to_string(),
                context_window: 32_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
            ModelInfo {
                id: "mixtral".to_string(),
                name: "Mixtral 8x7B".to_string(),
                provider: "local".to_string(),
                context_window: 32_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
            ModelInfo {
                id: "qwen2.5-coder".to_string(),
                name: "Qwen 2.5 Coder".to_string(),
                provider: "local".to_string(),
                context_window: 32_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
            ModelInfo {
                id: "deepseek-coder-v2".to_string(),
                name: "DeepSeek Coder V2".to_string(),
                provider: "local".to_string(),
                context_window: 128_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                pricing: None,
            },
        ]
    }

    fn is_configured(&self) -> bool {
        // For local, we consider it configured if the URL is set
        // Actual availability is checked via is_running()
        true
    }

    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        debug!("Sending request to Ollama");
        self.inner.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        debug!("Sending streaming request to Ollama");
        self.inner.complete_stream(request).await
    }
}

impl ToolCapable for LocalProvider {
    fn format_tools(&self, tools: &[ToolDefinition]) -> serde_json::Value {
        self.inner.format_tools(tools)
    }

    fn parse_tool_calls(&self, response: &CompletionResponse) -> anyhow::Result<Vec<ToolCall>> {
        self.inner.parse_tool_calls(response)
    }

    fn format_tool_results(&self, results: &[ToolResult]) -> Vec<Message> {
        self.inner.format_tool_results(results)
    }
}

// Ollama-specific types

#[derive(Debug, Deserialize)]
struct OllamaTags {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub modified_at: String,
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaModelDetails {
    pub format: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let provider = LocalProvider::new();
        assert_eq!(provider.id(), "local");
        assert_eq!(provider.name(), "Local (Ollama)");
        assert!(provider.is_configured());

        let models = provider.available_models();
        assert!(!models.is_empty());
        // Local models should have no pricing
        assert!(models.iter().all(|m| m.pricing.is_none()));
    }

    #[test]
    fn test_custom_url() {
        let provider = LocalProvider::with_url("http://192.168.1.100:11434/v1");
        assert_eq!(provider.base_url, "http://192.168.1.100:11434/v1");
    }
}
