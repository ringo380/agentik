//! Message and conversation primitives.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Assistant response
    Assistant,
    /// Tool result
    Tool,
}

/// Message content types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    /// Plain text content
    Text(String),
    /// Multiple content parts (for multimodal)
    Parts(Vec<ContentPart>),
}

impl Content {
    /// Create text content.
    pub fn text(s: impl Into<String>) -> Self {
        Content::Text(s.into())
    }

    /// Get content as text (concatenates parts if needed).
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Content part for multimodal messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    /// Text content
    #[serde(rename = "text")]
    Text { text: String },
    /// Image content
    #[serde(rename = "image")]
    Image {
        /// Base64-encoded image data or URL
        source: ImageSource,
    },
    /// Tool use request
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool result
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// Image source for multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    /// Base64-encoded image
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    /// URL reference
    #[serde(rename = "url")]
    Url { url: String },
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message ID
    pub id: String,
    /// Message role
    pub role: Role,
    /// Message content
    pub content: Content,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Token count (approximate)
    pub token_count: Option<u32>,
    /// Tool calls in this message (for assistant messages)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<super::ToolCall>,
}

impl Message {
    /// Create a new user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::User,
            content: Content::text(content),
            timestamp: Utc::now(),
            token_count: None,
            tool_calls: vec![],
        }
    }

    /// Create a new assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::Assistant,
            content: Content::text(content),
            timestamp: Utc::now(),
            token_count: None,
            tool_calls: vec![],
        }
    }

    /// Create a new system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::System,
            content: Content::text(content),
            timestamp: Utc::now(),
            token_count: None,
            tool_calls: vec![],
        }
    }

    /// Create a tool result message.
    pub fn tool_result(tool_use_id: String, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::Tool,
            content: Content::Parts(vec![ContentPart::ToolResult {
                tool_use_id,
                content: content.into(),
                is_error,
            }]),
            timestamp: Utc::now(),
            token_count: None,
            tool_calls: vec![],
        }
    }
}
