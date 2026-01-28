//! OpenAI (GPT) provider implementation.

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

/// Default OpenAI API base URL.
const OPENAI_API_URL: &str = "https://api.openai.com/v1";

/// OpenAI provider for GPT models.
pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
    default_model: String,
    organization: Option<String>,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: OPENAI_API_URL.to_string(),
            default_model: "gpt-4o".to_string(),
            organization: None,
        }
    }

    /// Create from environment variable.
    pub fn from_env() -> Option<Self> {
        std::env::var("OPENAI_API_KEY")
            .ok()
            .map(|key| Self::new(key))
    }

    /// Set a custom base URL (for OpenRouter, Azure, etc.).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set the default model.
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    /// Set the organization ID.
    pub fn with_organization(mut self, org: impl Into<String>) -> Self {
        self.organization = Some(org.into());
        self
    }

    /// Convert internal messages to OpenAI format.
    fn format_messages(&self, messages: &[Message]) -> Vec<OpenAIMessage> {
        messages.iter().map(|m| self.convert_message(m)).collect()
    }

    /// Convert a single message to OpenAI format.
    fn convert_message(&self, message: &Message) -> OpenAIMessage {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        };

        let content = match &message.content {
            agentik_core::Content::Text(text) => OpenAIContent::Text(text.clone()),
            agentik_core::Content::Parts(parts) => OpenAIContent::Parts(
                parts
                    .iter()
                    .filter_map(|p| self.convert_content_part(p))
                    .collect(),
            ),
        };

        // Handle tool calls for assistant messages
        let tool_calls = if !message.tool_calls.is_empty() {
            Some(
                message
                    .tool_calls
                    .iter()
                    .map(|tc| OpenAIToolCall {
                        id: tc.id.clone(),
                        tool_type: "function".to_string(),
                        function: OpenAIFunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                    })
                    .collect(),
            )
        } else {
            None
        };

        // Handle tool result messages
        let tool_call_id = if message.role == Role::Tool {
            self.extract_tool_call_id(&message.content)
        } else {
            None
        };

        OpenAIMessage {
            role: role.to_string(),
            content: Some(content),
            tool_calls,
            tool_call_id,
            name: None,
        }
    }

    /// Extract tool call ID from content.
    fn extract_tool_call_id(&self, content: &agentik_core::Content) -> Option<String> {
        match content {
            agentik_core::Content::Parts(parts) => {
                for part in parts {
                    if let agentik_core::message::ContentPart::ToolResult { tool_use_id, .. } = part
                    {
                        return Some(tool_use_id.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Convert content part to OpenAI format.
    fn convert_content_part(
        &self,
        part: &agentik_core::message::ContentPart,
    ) -> Option<OpenAIContentPart> {
        match part {
            agentik_core::message::ContentPart::Text { text } => {
                Some(OpenAIContentPart::Text { text: text.clone() })
            }
            agentik_core::message::ContentPart::Image { source } => {
                let url = match source {
                    agentik_core::message::ImageSource::Base64 { media_type, data } => {
                        format!("data:{};base64,{}", media_type, data)
                    }
                    agentik_core::message::ImageSource::Url { url } => url.clone(),
                };
                Some(OpenAIContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url,
                        detail: Some("auto".to_string()),
                    },
                })
            }
            agentik_core::message::ContentPart::ToolResult { content, .. } => {
                Some(OpenAIContentPart::Text {
                    text: content.clone(),
                })
            }
            _ => None,
        }
    }

    /// Convert tools to OpenAI format.
    fn format_tools_internal(&self, tools: &[ToolDefinition]) -> Vec<OpenAITool> {
        tools
            .iter()
            .map(|t| OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    }

    /// Parse response into our format.
    fn parse_response(&self, response: OpenAIResponse) -> CompletionResponse {
        let choice = response.choices.first();

        let content = choice
            .and_then(|c| c.message.content.as_ref())
            .map(|c| match c {
                OpenAIContent::Text(t) => t.clone(),
                OpenAIContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| match p {
                        OpenAIContentPart::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            })
            .unwrap_or_default();

        let tool_calls = choice
            .and_then(|c| c.message.tool_calls.as_ref())
            .map(|tcs| {
                tcs.iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::Value::Null),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = choice
            .and_then(|c| c.finish_reason.as_deref())
            .map(|r| match r {
                "stop" => FinishReason::Stop,
                "length" => FinishReason::MaxTokens,
                "tool_calls" => FinishReason::ToolUse,
                "content_filter" => FinishReason::ContentFilter,
                _ => FinishReason::Stop,
            })
            .unwrap_or(FinishReason::Stop);

        let usage = response
            .usage
            .map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                cached_tokens: u
                    .prompt_tokens_details
                    .map(|d| d.cached_tokens)
                    .unwrap_or(0),
            })
            .unwrap_or_default();

        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn name(&self) -> &str {
        "OpenAI"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
                max_output_tokens: 16_384,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 2.50,
                    output_per_million: 10.0,
                }),
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
                max_output_tokens: 16_384,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 0.15,
                    output_per_million: 0.60,
                }),
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
                max_output_tokens: 4_096,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 10.0,
                    output_per_million: 30.0,
                }),
            },
            ModelInfo {
                id: "o1".to_string(),
                name: "o1".to_string(),
                provider: "openai".to_string(),
                context_window: 200_000,
                max_output_tokens: 100_000,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 15.0,
                    output_per_million: 60.0,
                }),
            },
            ModelInfo {
                id: "o1-mini".to_string(),
                name: "o1-mini".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
                max_output_tokens: 65_536,
                supports_tools: true,
                supports_vision: true,
                supports_streaming: true,
                pricing: Some(Pricing {
                    input_per_million: 3.0,
                    output_per_million: 12.0,
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

        let mut messages = Vec::new();

        // Add system message if provided
        if let Some(ref system) = request.system {
            messages.push(OpenAIMessage {
                role: "system".to_string(),
                content: Some(OpenAIContent::Text(system.clone())),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        messages.extend(self.format_messages(&request.messages));

        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(self.format_tools_internal(&request.tools))
        };

        let api_request = OpenAIRequest {
            model: model.to_string(),
            messages,
            max_tokens: Some(request.max_tokens),
            temperature: Some(request.temperature),
            tools,
            stream: false,
            stop: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        debug!("Sending request to OpenAI API");

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(ref org) = self.organization {
            req = req.header("OpenAI-Organization", org);
        }

        let response = req.json(&api_request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("OpenAI API error: {} - {}", status, error_text);
            anyhow::bail!("OpenAI API error: {} - {}", status, error_text);
        }

        let api_response: OpenAIResponse = response.json().await?;
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

        let mut messages = Vec::new();

        if let Some(ref system) = request.system {
            messages.push(OpenAIMessage {
                role: "system".to_string(),
                content: Some(OpenAIContent::Text(system.clone())),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        messages.extend(self.format_messages(&request.messages));

        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(self.format_tools_internal(&request.tools))
        };

        let api_request = OpenAIRequest {
            model: model.to_string(),
            messages,
            max_tokens: Some(request.max_tokens),
            temperature: Some(request.temperature),
            tools,
            stream: true,
            stop: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        debug!("Sending streaming request to OpenAI API");

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(ref org) = self.organization {
            req = req.header("OpenAI-Organization", org);
        }

        let response = req.json(&api_request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("OpenAI API error: {} - {}", status, error_text);
            anyhow::bail!("OpenAI API error: {} - {}", status, error_text);
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

                                // Parse the event data as OpenAI stream event
                                match parse_openai_event(&event.data) {
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

impl ToolCapable for OpenAIProvider {
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

/// Parse an OpenAI stream event from JSON data.
fn parse_openai_event(data: &str) -> anyhow::Result<Option<StreamChunk>> {
    let chunk: StreamChunkResponse = serde_json::from_str(data)?;

    if let Some(choice) = chunk.choices.first() {
        let mut delta_text = None;
        let mut tool_call_delta = None;
        let is_final = choice.finish_reason.is_some();

        if let Some(ref d) = choice.delta {
            // Extract content delta
            if let Some(ref content) = d.content {
                delta_text = Some(content.clone());
            }

            // Extract tool call delta
            if let Some(ref tcs) = d.tool_calls {
                if let Some(tc) = tcs.first() {
                    tool_call_delta = Some(ToolCallDelta {
                        id: tc.id.clone(),
                        name: tc.function.as_ref().and_then(|f| f.name.clone()),
                        arguments: tc.function.as_ref().and_then(|f| f.arguments.clone()),
                    });
                }
            }
        }

        // Only return a chunk if we have something to emit
        if delta_text.is_some() || tool_call_delta.is_some() || is_final {
            Ok(Some(StreamChunk {
                delta: delta_text,
                tool_call_delta,
                is_final,
                usage: None,
            }))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

// OpenAI API types

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAIContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAIContent {
    Text(String),
    Parts(Vec<OpenAIContentPart>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum OpenAIContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize, Deserialize)]
struct ImageUrl {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAITool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunction,
}

#[derive(Debug, Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    cached_tokens: u32,
}

// Streaming types

#[derive(Debug, Deserialize)]
struct StreamChunkResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCall {
    id: Option<String>,
    function: Option<StreamFunction>,
}

#[derive(Debug, Deserialize)]
struct StreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let provider = OpenAIProvider::new("test-key");
        assert_eq!(provider.id(), "openai");
        assert_eq!(provider.name(), "OpenAI");
        assert!(provider.is_configured());

        let models = provider.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "gpt-4o"));
    }

    #[test]
    fn test_custom_base_url() {
        let provider =
            OpenAIProvider::new("test-key").with_base_url("https://openrouter.ai/api/v1");
        assert_eq!(provider.base_url, "https://openrouter.ai/api/v1");
    }
}
