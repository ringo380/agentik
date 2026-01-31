//! MCP tool wrapper for integration with agentik's tool system.
//!
//! This module provides `McpToolWrapper`, which wraps MCP tools to implement
//! the `Tool` trait from `agentik-tools`, allowing them to be used alongside
//! built-in tools.

use std::sync::Arc;

use agentik_core::tool::{ToolCategory, ToolDefinition};
use agentik_core::{ToolCall, ToolResult};
use agentik_tools::{Tool, ToolContext, ToolError};
use async_trait::async_trait;

use crate::client::McpClient;
use crate::protocol::McpToolDefinition;

/// Prefix for MCP tool names to avoid collisions with built-in tools.
pub const MCP_TOOL_PREFIX: &str = "mcp__";

/// Wrapper that exposes an MCP tool as an agentik `Tool`.
pub struct McpToolWrapper {
    /// Full tool name including server prefix (e.g., "mcp__filesystem__read_file").
    full_name: String,
    /// Server name.
    server_name: String,
    /// Original tool name on the MCP server.
    tool_name: String,
    /// MCP tool definition.
    tool_def: McpToolDefinition,
    /// Reference to the MCP client.
    client: Arc<McpClient>,
}

impl McpToolWrapper {
    /// Create a new MCP tool wrapper.
    ///
    /// The tool name will be prefixed with `mcp__<server>__` to avoid collisions.
    pub fn new(
        server_name: impl Into<String>,
        tool_def: McpToolDefinition,
        client: Arc<McpClient>,
    ) -> Self {
        let server_name = server_name.into();
        let tool_name = tool_def.name.clone();
        let full_name = format!("{}{}__{}", MCP_TOOL_PREFIX, server_name, tool_name);

        Self {
            full_name,
            server_name,
            tool_name,
            tool_def,
            client,
        }
    }

    /// Get the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Get the original tool name.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Parse a full MCP tool name into (server_name, tool_name).
    ///
    /// Returns None if the name is not a valid MCP tool name.
    pub fn parse_tool_name(full_name: &str) -> Option<(String, String)> {
        if !full_name.starts_with(MCP_TOOL_PREFIX) {
            return None;
        }

        let rest = &full_name[MCP_TOOL_PREFIX.len()..];
        let parts: Vec<&str> = rest.splitn(2, "__").collect();

        if parts.len() == 2 {
            Some((parts[0].to_string(), parts[1].to_string()))
        } else {
            None
        }
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn definition(&self) -> ToolDefinition {
        let description = self
            .tool_def
            .description
            .clone()
            .unwrap_or_else(|| format!("MCP tool from {}", self.server_name));

        ToolDefinition::new(&self.full_name, description)
            .with_parameters(self.tool_def.input_schema.clone())
            .with_category(ToolCategory::Mcp(self.server_name.clone()))
    }

    async fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = std::time::Instant::now();

        // Call the MCP tool
        let mcp_result = self
            .client
            .call_tool(&self.server_name, &self.tool_name, Some(call.arguments.clone()))
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Convert MCP result to agentik ToolResult
        if mcp_result.is_error {
            // Collect error text from content
            let error_message = mcp_result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .collect::<Vec<_>>()
                .join("\n");

            Ok(ToolResult::error(&call.id, error_message).with_duration(duration_ms))
        } else {
            // Collect output text from content
            let output = mcp_result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .collect::<Vec<_>>()
                .join("\n");

            Ok(ToolResult::success(&call.id, output).with_duration(duration_ms))
        }
    }
}

/// Create MCP tool wrappers from a client.
///
/// This function creates `McpToolWrapper` instances for all tools available
/// from the connected MCP servers.
pub async fn create_mcp_tools(client: Arc<McpClient>) -> Vec<McpToolWrapper> {
    let all_tools = client.all_tools().await;
    all_tools
        .into_iter()
        .map(|(server_name, tool_def)| McpToolWrapper::new(server_name, tool_def, Arc::clone(&client)))
        .collect()
}

/// Register all MCP tools with a tool registry.
pub async fn register_mcp_tools(
    client: Arc<McpClient>,
    registry: &mut agentik_tools::ToolRegistry,
) {
    let tools = create_mcp_tools(client).await;
    for tool in tools {
        registry.register(Arc::new(tool));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mcp_tool_name_generation() {
        let client = Arc::new(McpClient::new());
        let tool_def = McpToolDefinition {
            name: "read_file".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        };

        let wrapper = McpToolWrapper::new("filesystem", tool_def, client);

        assert_eq!(wrapper.name(), "mcp__filesystem__read_file");
        assert_eq!(wrapper.server_name(), "filesystem");
        assert_eq!(wrapper.tool_name(), "read_file");
    }

    #[test]
    fn test_parse_tool_name() {
        let result = McpToolWrapper::parse_tool_name("mcp__filesystem__read_file");
        assert_eq!(result, Some(("filesystem".to_string(), "read_file".to_string())));

        let result = McpToolWrapper::parse_tool_name("mcp__github__create_issue");
        assert_eq!(result, Some(("github".to_string(), "create_issue".to_string())));

        // Invalid names
        let result = McpToolWrapper::parse_tool_name("not_mcp_tool");
        assert_eq!(result, None);

        let result = McpToolWrapper::parse_tool_name("mcp__only_server");
        assert_eq!(result, None);
    }

    #[test]
    fn test_tool_definition_generation() {
        let client = Arc::new(McpClient::new());
        let tool_def = McpToolDefinition {
            name: "list_files".to_string(),
            description: Some("List files in a directory".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "directory": {"type": "string"}
                },
                "required": ["directory"]
            }),
        };

        let wrapper = McpToolWrapper::new("fs", tool_def, client);
        let definition = wrapper.definition();

        assert_eq!(definition.name, "mcp__fs__list_files");
        assert_eq!(definition.description, "List files in a directory");
        assert!(matches!(definition.category, ToolCategory::Mcp(ref s) if s == "fs"));
    }
}
