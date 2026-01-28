//! Provider trait definitions.

use agentik_core::{Message, ToolCall, ToolDefinition};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use futures::Stream;

/// Model information with capabilities and pricing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Provider name
    pub provider: String,
    /// Context window size in tokens
    pub context_window: u32,
    /// Maximum output tokens
    pub max_output_tokens: u32,
    /// Supports tool/function calling
    pub supports_tools: bool,
    /// Supports vision/images
    pub supports_vision: bool,
    /// Supports streaming
    pub supports_streaming: bool,
    /// Pricing information
    pub pricing: Option<Pricing>,
}

/// Pricing information for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pricing {
    /// Cost per million input tokens (USD)
    pub input_per_million: f64,
    /// Cost per million output tokens (USD)
    pub output_per_million: f64,
}

/// Request for a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Model to use
    pub model: String,
    /// Messages in the conversation
    pub messages: Vec<Message>,
    /// System prompt
    pub system: Option<String>,
    /// Maximum tokens to generate
    pub max_tokens: u32,
    /// Temperature (0.0-1.0)
    pub temperature: f32,
    /// Available tools
    pub tools: Vec<ToolDefinition>,
    /// Stop sequences
    #[serde(default)]
    pub stop: Vec<String>,
}

/// Response from a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Response content
    pub content: String,
    /// Tool calls requested
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Finish reason
    pub finish_reason: FinishReason,
    /// Usage statistics
    pub usage: Usage,
}

/// Reason the completion finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Normal completion
    Stop,
    /// Hit max tokens limit
    MaxTokens,
    /// Tool use requested
    ToolUse,
    /// Content was filtered
    ContentFilter,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens used
    pub input_tokens: u32,
    /// Output tokens generated
    pub output_tokens: u32,
    /// Cached tokens (if applicable)
    pub cached_tokens: u32,
}

/// Streaming chunk from a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    /// Content delta
    pub delta: Option<String>,
    /// Tool call delta
    pub tool_call_delta: Option<ToolCallDelta>,
    /// Whether this is the final chunk
    pub is_final: bool,
    /// Usage (only in final chunk)
    pub usage: Option<Usage>,
}

/// Delta for tool call streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDelta {
    /// Tool call ID
    pub id: Option<String>,
    /// Tool name
    pub name: Option<String>,
    /// Arguments delta (JSON string fragment)
    pub arguments: Option<String>,
}

/// Core provider trait - all AI providers implement this.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider identifier.
    fn id(&self) -> &str;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Get available models for this provider.
    fn available_models(&self) -> Vec<ModelInfo>;

    /// Check if provider is configured and ready.
    fn is_configured(&self) -> bool;

    /// Generate a completion (non-streaming).
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse>;

    /// Generate a completion (streaming).
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>>;
}

/// Tool calling capability - providers that support function calling.
pub trait ToolCapable: Provider {
    /// Convert internal tool definitions to provider-specific format.
    fn format_tools(&self, tools: &[ToolDefinition]) -> serde_json::Value;

    /// Parse tool calls from provider response.
    fn parse_tool_calls(&self, response: &CompletionResponse) -> anyhow::Result<Vec<ToolCall>>;

    /// Format tool results for the provider.
    fn format_tool_results(&self, results: &[agentik_core::ToolResult]) -> Vec<Message>;
}
