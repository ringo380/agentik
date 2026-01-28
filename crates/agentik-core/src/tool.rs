//! Tool definitions and execution types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tool category for organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// File system operations
    FileSystem,
    /// Shell/command execution
    Shell,
    /// Git operations
    Git,
    /// Web operations (fetch, search)
    Web,
    /// External application integration
    External,
    /// MCP-provided tool
    Mcp(String),
}

/// Definition of a tool available to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name (unique identifier)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for parameters
    pub parameters: serde_json::Value,
    /// Tool category
    pub category: ToolCategory,
    /// Whether this tool requires user approval
    pub requires_approval: bool,
    /// Whether this tool is destructive (modifies state)
    pub is_destructive: bool,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            category: ToolCategory::External,
            requires_approval: false,
            is_destructive: false,
        }
    }

    /// Set the parameters schema.
    pub fn with_parameters(mut self, schema: serde_json::Value) -> Self {
        self.parameters = schema;
        self
    }

    /// Set the category.
    pub fn with_category(mut self, category: ToolCategory) -> Self {
        self.category = category;
        self
    }

    /// Mark as requiring approval.
    pub fn requires_approval(mut self) -> Self {
        self.requires_approval = true;
        self
    }

    /// Mark as destructive.
    pub fn destructive(mut self) -> Self {
        self.is_destructive = true;
        self
    }
}

/// A request to call a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call
    pub id: String,
    /// Tool name
    pub name: String,
    /// Tool arguments
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// ID of the tool call this is responding to
    pub tool_call_id: String,
    /// Whether execution succeeded
    pub success: bool,
    /// Output content
    pub output: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Artifacts produced (files, etc.)
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

impl ToolResult {
    /// Create a successful result.
    pub fn success(tool_call_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            success: true,
            output: output.into(),
            error: None,
            artifacts: vec![],
            duration_ms: 0,
        }
    }

    /// Create a failed result.
    pub fn error(tool_call_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            success: false,
            output: String::new(),
            error: Some(error.into()),
            artifacts: vec![],
            duration_ms: 0,
        }
    }

    /// Add an artifact.
    pub fn with_artifact(mut self, artifact: Artifact) -> Self {
        self.artifacts.push(artifact);
        self
    }

    /// Set the duration.
    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

/// Artifact produced by a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Artifact type
    pub artifact_type: ArtifactType,
    /// Artifact name/path
    pub name: String,
    /// Artifact content or reference
    pub content: String,
    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Type of artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    /// File created or modified
    File,
    /// Diff/patch
    Diff,
    /// Screenshot or image
    Image,
    /// Log output
    Log,
    /// Other artifact
    Other,
}
