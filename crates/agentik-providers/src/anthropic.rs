//! Anthropic (Claude) provider implementation.

use std::pin::Pin;

use async_trait::async_trait;
use futures::{stream, Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, instrument, warn};

use agentik_core::{Message, Role, ToolCall, ToolDefinition, ToolResult};

use crate::sse::SseParser;
use crate::traits::{
    CompletionRequest, CompletionResponse, FinishReason, ModelInfo, Pricing, Provider, StreamChunk,
    ToolCallDelta, ToolCapable, Usage,
};

/// Anthropic API base URL.
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1";

/// Current Anthropic API version.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic provider for Claude models.
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    default_model: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            default_model: "claude-sonnet-4-20250514".to_string(),
        }
    }

    /// Create from environment variable.
    pub fn from_env() -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY").ok().map(Self::new)
    }

    /// Set the default model.
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    /// Convert internal messages to Anthropic format.
    fn format_messages(&self, messages: &[Message]) -> Vec<AnthropicMessage> {
        messages
            .iter()
            .filter(|m| m.role != Role::System) // System handled separately
            .map(|m| self.convert_message(m))
            .collect()
    }

    /// Convert a single message to Anthropic format.
    fn convert_message(&self, message: &Message) -> AnthropicMessage {
        let role = match message.role {
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
            Role::System => "user", // Should be filtered out
        };

        let content = match &message.content {
            agentik_core::Content::Text(text) => {
                vec![AnthropicContent::Text { text: text.clone() }]
            }
            agentik_core::Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| self.convert_content_part(p))
                .collect(),
        };

        // Add tool results if this is a tool message
        let content = if message.role == Role::Tool {
            self.format_tool_results_content(message)
        } else {
            content
        };

        AnthropicMessage {
            role: role.to_string(),
            content,
        }
    }

    /// Convert content part to Anthropic format.
    fn convert_content_part(
        &self,
        part: &agentik_core::message::ContentPart,
    ) -> Option<AnthropicContent> {
        match part {
            agentik_core::message::ContentPart::Text { text } => {
                Some(AnthropicContent::Text { text: text.clone() })
            }
            agentik_core::message::ContentPart::Image { source } => {
                match source {
                    agentik_core::message::ImageSource::Base64 { media_type, data } => {
                        Some(AnthropicContent::Image {
                            source: ImageSource {
                                source_type: "base64".to_string(),
                                media_type: media_type.clone(),
                                data: data.clone(),
                            },
                        })
                    }
                    agentik_core::message::ImageSource::Url { url } => {
                        // Anthropic doesn't support URL images directly
                        // Would need to fetch and convert to base64
                        debug!("URL images not directly supported, skipping: {}", url);
                        None
                    }
                }
            }
            agentik_core::message::ContentPart::ToolUse { id, name, input } => {
                Some(AnthropicContent::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                })
            }
            agentik_core::message::ContentPart::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(AnthropicContent::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
            }),
        }
    }

    /// Format tool results as content blocks.
    fn format_tool_results_content(&self, message: &Message) -> Vec<AnthropicContent> {
        match &message.content {
            agentik_core::Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| self.convert_content_part(p))
                .collect(),
            agentik_core::Content::Text(text) => {
                vec![AnthropicContent::Text { text: text.clone() }]
            }
        }
    }

    /// Extract system prompt from messages.
    fn extract_system(
        &self,
        messages: &[Message],
        explicit_system: Option<&str>,
    ) -> Option<String> {
        if let Some(sys) = explicit_system {
            return Some(sys.to_string());
        }

        messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.as_text())
    }

    /// Convert tools to Anthropic format.
    fn format_tools_internal(&self, tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
        tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect()
    }

    /// Parse response into our format.
    fn parse_response(&self, response: AnthropicResponse) -> CompletionResponse {
        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for block in &response.content {
            match block {
                AnthropicContent::Text { text } => {
                    content.push_str(text);
                }
                AnthropicContent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: input.clone(),
                    });
                }
                _ => {}
            }
        }

        let finish_reason = match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::MaxTokens,
            Some("tool_use") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        };

        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
            usage: Usage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
                cached_tokens: response.usage.cache_read_input_tokens.unwrap_or(0),
            },
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn name(&self) -> &str {
        "Anthropic"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "claude-opus-4-20250514".to_string(),
                name: "Claude Opus 4".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200_000,
                max_output_tokens: 32_000,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 15.0,
                    output_per_million: 75.0,
                }),
            },
            ModelInfo {
                id: "claude-sonnet-4-20250514".to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200_000,
                max_output_tokens: 64_000,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 3.0,
                    output_per_million: 15.0,
                }),
            },
            ModelInfo {
                id: "claude-haiku-3-5-20241022".to_string(),
                name: "Claude 3.5 Haiku".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 0.80,
                    output_per_million: 4.0,
                }),
            },
        ]
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let model = if request.model.is_empty() {
            &self.default_model
        } else {
            &request.model
        };

        let system = self.extract_system(&request.messages, request.system.as_deref());
        let messages = self.format_messages(&request.messages);
        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(self.format_tools_internal(&request.tools))
        };

        let api_request = AnthropicRequest {
            model: model.to_string(),
            messages,
            system,
            max_tokens: request.max_tokens,
            temperature: Some(request.temperature),
            tools,
            stream: false,
            stop_sequences: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        debug!("Sending request to Anthropic API");

        let response = self
            .client
            .post(format!("{}/messages", ANTHROPIC_API_URL))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&api_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Anthropic API error: {} - {}", status, error_text);
            anyhow::bail!("Anthropic API error: {} - {}", status, error_text);
        }

        let api_response: AnthropicResponse = response.json().await?;
        Ok(self.parse_response(api_response))
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let model = if request.model.is_empty() {
            &self.default_model
        } else {
            &request.model
        };

        let system = self.extract_system(&request.messages, request.system.as_deref());
        let messages = self.format_messages(&request.messages);
        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(self.format_tools_internal(&request.tools))
        };

        let api_request = AnthropicRequest {
            model: model.to_string(),
            messages,
            system,
            max_tokens: request.max_tokens,
            temperature: Some(request.temperature),
            tools,
            stream: true,
            stop_sequences: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        debug!("Sending streaming request to Anthropic API");

        let response = self
            .client
            .post(format!("{}/messages", ANTHROPIC_API_URL))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&api_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Anthropic API error: {} - {}", status, error_text);
            anyhow::bail!("Anthropic API error: {} - {}", status, error_text);
        }

        let byte_stream = response.bytes_stream();

        // Use stateful SSE parser to handle line buffering across TCP chunks
        let parsed_stream = stream::unfold(
            (byte_stream, SseParser::new()),
            |(mut byte_stream, mut parser)| async move {
                loop {
                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            let events = parser.feed(&bytes);

                            // Process all events from this chunk
                            for event in events {
                                if event.is_done() {
                                    // Return final chunk
                                    return Some((
                                        Ok(StreamChunk {
                                            delta: None,
                                            tool_call_delta: None,
                                            is_final: true,
                                            usage: None,
                                        }),
                                        (byte_stream, parser),
                                    ));
                                }

                                // Parse the event data as Anthropic stream event
                                match parse_anthropic_event(&event.data) {
                                    Ok(Some(chunk)) => {
                                        return Some((Ok(chunk), (byte_stream, parser)));
                                    }
                                    Ok(None) => {
                                        // Event parsed but no content to emit, continue
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse SSE event: {}", e);
                                        // Continue processing
                                    }
                                }
                            }
                            // No events to emit from this chunk, continue reading
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(anyhow::anyhow!("Stream error: {}", e)),
                                (byte_stream, parser),
                            ));
                        }
                        None => {
                            // Stream ended
                            return None;
                        }
                    }
                }
            },
        );

        Ok(Box::pin(parsed_stream))
    }
}

impl ToolCapable for AnthropicProvider {
    fn format_tools(&self, tools: &[ToolDefinition]) -> serde_json::Value {
        serde_json::to_value(self.format_tools_internal(tools)).unwrap_or_default()
    }

    fn parse_tool_calls(&self, response: &CompletionResponse) -> anyhow::Result<Vec<ToolCall>> {
        Ok(response.tool_calls.clone())
    }

    fn format_tool_results(&self, results: &[ToolResult]) -> Vec<Message> {
        results
            .iter()
            .map(|r| Message::tool_result(r.tool_call_id.clone(), &r.output, !r.success))
            .collect()
    }
}

/// Parse an Anthropic stream event from JSON data.
fn parse_anthropic_event(data: &str) -> anyhow::Result<Option<StreamChunk>> {
    let event: StreamEvent = serde_json::from_str(data)?;

    match event {
        StreamEvent::ContentBlockDelta { delta: d, .. } => {
            let text_delta = d.text;
            let tool_delta = if d.partial_json.is_some() {
                Some(ToolCallDelta {
                    id: None,
                    name: None,
                    arguments: d.partial_json,
                })
            } else {
                None
            };

            if text_delta.is_some() || tool_delta.is_some() {
                Ok(Some(StreamChunk {
                    delta: text_delta,
                    tool_call_delta: tool_delta,
                    is_final: false,
                    usage: None,
                }))
            } else {
                Ok(None)
            }
        }
        StreamEvent::ContentBlockStart { content_block, .. } => {
            if content_block.block_type == "tool_use" {
                Ok(Some(StreamChunk {
                    delta: None,
                    tool_call_delta: Some(ToolCallDelta {
                        id: content_block.id,
                        name: content_block.name,
                        arguments: None,
                    }),
                    is_final: false,
                    usage: None,
                }))
            } else if content_block.block_type == "text" {
                // Text block start, might have initial text
                if let Some(text) = content_block.text {
                    Ok(Some(StreamChunk {
                        delta: Some(text),
                        tool_call_delta: None,
                        is_final: false,
                        usage: None,
                    }))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        }
        StreamEvent::MessageDelta { usage: u, .. } => {
            if let Some(u) = u {
                Ok(Some(StreamChunk {
                    delta: None,
                    tool_call_delta: None,
                    is_final: false,
                    usage: Some(Usage {
                        input_tokens: 0,
                        output_tokens: u.output_tokens,
                        cached_tokens: 0,
                    }),
                }))
            } else {
                Ok(None)
            }
        }
        StreamEvent::MessageStop => Ok(Some(StreamChunk {
            delta: None,
            tool_call_delta: None,
            is_final: true,
            usage: None,
        })),
        StreamEvent::MessageStart { message } => {
            // Extract input token count from message_start
            if let Some(msg) = message {
                if let Some(usage) = msg.get("usage") {
                    if let Some(input_tokens) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        return Ok(Some(StreamChunk {
                            delta: None,
                            tool_call_delta: None,
                            is_final: false,
                            usage: Some(Usage {
                                input_tokens: input_tokens as u32,
                                output_tokens: 0,
                                cached_tokens: 0,
                            }),
                        }));
                    }
                }
            }
            Ok(None)
        }
        StreamEvent::ContentBlockStop { .. } => Ok(None),
        StreamEvent::Ping => Ok(None),
        StreamEvent::Error { error } => Err(anyhow::anyhow!("Anthropic stream error: {:?}", error)),
    }
}

// Anthropic API types

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: Option<u32>,
}

// Streaming event types

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: Option<serde_json::Value> },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: ContentDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: Option<serde_json::Value>,
        usage: Option<DeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: serde_json::Value },
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    id: Option<String>,
    name: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ContentDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeltaUsage {
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let provider = AnthropicProvider::new("test-key");
        assert_eq!(provider.id(), "anthropic");
        assert_eq!(provider.name(), "Anthropic");
        assert!(provider.is_configured());

        let models = provider.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("sonnet")));
    }

    #[test]
    fn test_format_messages() {
        let provider = AnthropicProvider::new("test-key");

        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];

        let formatted = provider.format_messages(&messages);
        assert_eq!(formatted.len(), 2);
        assert_eq!(formatted[0].role, "user");
        assert_eq!(formatted[1].role, "assistant");
    }
}
