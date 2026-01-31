//! # agentik-mcp
//!
//! MCP (Model Context Protocol) client integration for Agentik.
//!
//! This crate provides a Rust implementation of the MCP client, allowing
//! Agentik to connect to MCP servers and use their tools.
//!
//! ## Features
//!
//! - **Multi-server support**: Connect to multiple MCP servers simultaneously
//! - **Stdio transport**: Communicate with servers via stdin/stdout
//! - **Tool discovery**: Automatically discover and register tools from servers
//! - **Integration**: Seamlessly integrates with agentik's tool system
//!
//! ## Usage
//!
//! ```ignore
//! use agentik_mcp::{McpServerManager, McpServerConfig};
//!
//! // Create a server manager
//! let manager = McpServerManager::new();
//!
//! // Add a server configuration
//! let config = McpServerConfig::new("filesystem", "npx")
//!     .with_args(vec!["-y".to_string(), "@anthropic/mcp-server-filesystem".to_string()]);
//! manager.add_config(config).await;
//!
//! // Start all servers
//! manager.start_all().await?;
//!
//! // Register tools with a tool registry
//! let mut registry = ToolRegistry::new();
//! manager.register_tools(&mut registry).await;
//! ```
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`error`]: MCP-specific error types
//! - [`protocol`]: JSON-RPC 2.0 and MCP message types
//! - [`transport`]: Transport implementations (stdio)
//! - [`client`]: MCP client and connection management
//! - [`tools`]: Tool wrapper for agentik integration
//! - [`discovery`]: Server lifecycle management

pub mod client;
pub mod discovery;
pub mod error;
pub mod protocol;
pub mod tools;
pub mod transport;

// Re-export main types for convenience
pub use client::{ConnectionState, McpClient, McpConnection, McpServerConfig};
pub use discovery::{McpServerManager, ServerStatus};
pub use error::{McpError, TransportError};
pub use protocol::{
    CallToolParams, CallToolResult, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ListToolsResult, McpToolDefinition,
    RequestId, ServerCapabilities, ServerInfo, ToolContent, MCP_PROTOCOL_VERSION,
};
pub use tools::{create_mcp_tools, register_mcp_tools, McpToolWrapper, MCP_TOOL_PREFIX};
pub use transport::{StdioTransport, Transport};
