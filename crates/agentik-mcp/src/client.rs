//! MCP client implementation.
//!
//! This module provides the `McpConnection` for managing individual server connections
//! and `McpClient` for managing multiple servers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::error::McpError;
use crate::protocol::{
    CallToolParams, CallToolResult, InitializeParams, InitializeResult, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, ListToolsResult, McpToolDefinition, ServerCapabilities,
};
use crate::transport::{StdioTransport, Transport};

/// Connection state for an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected.
    Disconnected,
    /// Connected but not initialized.
    Connected,
    /// Connection established and initialized.
    Ready,
    /// Connection is being closed.
    Closing,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "disconnected"),
            Self::Connected => write!(f, "connected"),
            Self::Ready => write!(f, "ready"),
            Self::Closing => write!(f, "closing"),
        }
    }
}

/// A connection to a single MCP server.
pub struct McpConnection {
    /// Server name.
    name: String,
    /// Transport for communication.
    transport: Mutex<Box<dyn Transport>>,
    /// Current connection state.
    state: RwLock<ConnectionState>,
    /// Server capabilities after initialization.
    server_capabilities: RwLock<Option<ServerCapabilities>>,
    /// Cached tools from the server.
    cached_tools: RwLock<Vec<McpToolDefinition>>,
    /// Request ID counter.
    request_counter: AtomicU64,
}

impl McpConnection {
    /// Create a new connection with an existing transport.
    pub fn new(name: String, transport: Box<dyn Transport>) -> Self {
        Self {
            name,
            transport: Mutex::new(transport),
            state: RwLock::new(ConnectionState::Connected),
            server_capabilities: RwLock::new(None),
            cached_tools: RwLock::new(Vec::new()),
            request_counter: AtomicU64::new(1),
        }
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the current connection state.
    pub async fn state(&self) -> ConnectionState {
        *self.state.read().await
    }

    /// Check if the connection is ready for use.
    pub async fn is_ready(&self) -> bool {
        *self.state.read().await == ConnectionState::Ready
    }

    /// Get the server capabilities.
    pub async fn capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.read().await.clone()
    }

    /// Get the cached tools.
    pub async fn tools(&self) -> Vec<McpToolDefinition> {
        self.cached_tools.read().await.clone()
    }

    /// Generate a new request ID.
    fn next_request_id(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a request and wait for the response.
    async fn request<P, R>(&self, method: &str, params: Option<P>) -> Result<R, McpError>
    where
        P: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        let mut transport = self.transport.lock().await;

        // Send the request
        transport.send(&request_json).await?;

        // Wait for the response
        let response_json = transport.receive().await?;

        // Parse the response
        let response: JsonRpcResponse<R> = serde_json::from_str(&response_json)
            .map_err(|e| McpError::protocol(format!("Failed to parse response: {}", e)))?;

        // Check for errors
        if let Some(error) = response.error {
            return Err(McpError::server_error(error.code, error.message));
        }

        // Extract the result
        response
            .result
            .ok_or_else(|| McpError::protocol("Response missing result"))
    }

    /// Send a notification (no response expected).
    async fn notify<P>(&self, method: &str, params: Option<P>) -> Result<(), McpError>
    where
        P: serde::Serialize,
    {
        let notification = JsonRpcNotification::new(method, params);
        let notification_json = serde_json::to_string(&notification)?;

        let mut transport = self.transport.lock().await;
        transport.send(&notification_json).await?;

        Ok(())
    }

    /// Initialize the connection with the server.
    pub async fn initialize(&self) -> Result<(), McpError> {
        let state = *self.state.read().await;
        if state != ConnectionState::Connected {
            return Err(McpError::invalid_state("connected", state.to_string()));
        }

        debug!(server = %self.name, "Initializing MCP connection");

        // Send initialize request
        let params = InitializeParams::default();
        let result: InitializeResult = self.request("initialize", Some(params)).await?;

        // Store server capabilities
        *self.server_capabilities.write().await = Some(result.capabilities);

        // Send initialized notification
        self.notify::<()>("notifications/initialized", None)
            .await?;

        // Update state
        *self.state.write().await = ConnectionState::Ready;

        info!(
            server = %self.name,
            server_name = %result.server_info.name,
            protocol_version = %result.protocol_version,
            "MCP connection initialized"
        );

        Ok(())
    }

    /// List available tools from the server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpError> {
        let state = *self.state.read().await;
        if state != ConnectionState::Ready {
            return Err(McpError::invalid_state("ready", state.to_string()));
        }

        debug!(server = %self.name, "Listing tools");

        let result: ListToolsResult = self.request::<(), _>("tools/list", None).await?;

        // Cache the tools
        *self.cached_tools.write().await = result.tools.clone();

        debug!(
            server = %self.name,
            tool_count = result.tools.len(),
            "Listed tools"
        );

        Ok(result.tools)
    }

    /// Call a tool on the server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResult, McpError> {
        let state = *self.state.read().await;
        if state != ConnectionState::Ready {
            return Err(McpError::invalid_state("ready", state.to_string()));
        }

        debug!(server = %self.name, tool = name, "Calling tool");

        let params = CallToolParams {
            name: name.to_string(),
            arguments,
        };

        let result: CallToolResult = self.request("tools/call", Some(params)).await?;

        if result.is_error {
            warn!(
                server = %self.name,
                tool = name,
                "Tool returned error"
            );
        }

        Ok(result)
    }

    /// Close the connection.
    pub async fn close(&self) -> Result<(), McpError> {
        let state = *self.state.read().await;
        if state == ConnectionState::Disconnected {
            return Ok(());
        }

        *self.state.write().await = ConnectionState::Closing;

        debug!(server = %self.name, "Closing MCP connection");

        let mut transport = self.transport.lock().await;
        transport.close().await?;

        *self.state.write().await = ConnectionState::Disconnected;

        info!(server = %self.name, "MCP connection closed");

        Ok(())
    }
}

/// Configuration for an MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique server name.
    pub name: String,
    /// Command to execute.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Working directory.
    pub working_dir: Option<std::path::PathBuf>,
    /// Whether this server is enabled.
    pub enabled: bool,
}

impl McpServerConfig {
    /// Create a new server configuration.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
            enabled: true,
        }
    }

    /// Add arguments.
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Add an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Set whether the server is enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Client for managing multiple MCP server connections.
pub struct McpClient {
    /// Active connections by server name.
    connections: RwLock<HashMap<String, Arc<McpConnection>>>,
}

impl McpClient {
    /// Create a new MCP client.
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Connect to an MCP server.
    pub async fn connect(&self, config: McpServerConfig) -> Result<Arc<McpConnection>, McpError> {
        if !config.enabled {
            return Err(McpError::InitializationFailed(format!(
                "Server '{}' is disabled",
                config.name
            )));
        }

        // Check if already connected
        {
            let connections = self.connections.read().await;
            if connections.contains_key(&config.name) {
                return Err(McpError::ServerAlreadyExists(config.name.clone()));
            }
        }

        info!(
            server = %config.name,
            command = %config.command,
            "Connecting to MCP server"
        );

        // Spawn the transport
        let transport = StdioTransport::spawn(
            &config.command,
            &config.args,
            config.env,
            config.working_dir.as_ref(),
        )
        .await?;

        // Create the connection
        let connection = Arc::new(McpConnection::new(config.name.clone(), Box::new(transport)));

        // Initialize the connection
        connection.initialize().await?;

        // Store the connection
        {
            let mut connections = self.connections.write().await;
            connections.insert(config.name, Arc::clone(&connection));
        }

        Ok(connection)
    }

    /// Disconnect from an MCP server.
    pub async fn disconnect(&self, name: &str) -> Result<(), McpError> {
        let connection = {
            let mut connections = self.connections.write().await;
            connections
                .remove(name)
                .ok_or_else(|| McpError::ServerNotFound(name.to_string()))?
        };

        connection.close().await
    }

    /// Disconnect from all servers.
    pub async fn disconnect_all(&self) -> Result<(), McpError> {
        let connections: Vec<Arc<McpConnection>> = {
            let mut connections = self.connections.write().await;
            connections.drain().map(|(_, conn)| conn).collect()
        };

        for connection in connections {
            if let Err(e) = connection.close().await {
                error!(
                    server = %connection.name(),
                    error = %e,
                    "Failed to close connection"
                );
            }
        }

        Ok(())
    }

    /// Get a connection by name.
    pub async fn get(&self, name: &str) -> Option<Arc<McpConnection>> {
        self.connections.read().await.get(name).cloned()
    }

    /// List all connected server names.
    pub async fn list_servers(&self) -> Vec<String> {
        self.connections.read().await.keys().cloned().collect()
    }

    /// Get all tools from all connected servers.
    ///
    /// Returns a list of (server_name, tool_definition) tuples.
    pub async fn all_tools(&self) -> Vec<(String, McpToolDefinition)> {
        let connections = self.connections.read().await;
        let mut tools = Vec::new();

        for (server_name, connection) in connections.iter() {
            for tool in connection.tools().await {
                tools.push((server_name.clone(), tool));
            }
        }

        tools
    }

    /// Refresh tools from all connected servers.
    pub async fn refresh_tools(&self) -> Result<(), McpError> {
        let connections: Vec<Arc<McpConnection>> = {
            self.connections.read().await.values().cloned().collect()
        };

        for connection in connections {
            if connection.is_ready().await {
                if let Err(e) = connection.list_tools().await {
                    error!(
                        server = %connection.name(),
                        error = %e,
                        "Failed to refresh tools"
                    );
                }
            }
        }

        Ok(())
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResult, McpError> {
        let connection = self
            .get(server)
            .await
            .ok_or_else(|| McpError::ServerNotFound(server.to_string()))?;

        connection.call_tool(tool, arguments).await
    }
}

impl Default for McpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_builder() {
        let config = McpServerConfig::new("test", "npx")
            .with_args(vec!["-y".to_string(), "@anthropic/mcp-server".to_string()])
            .with_env("NODE_ENV", "production")
            .with_enabled(true);

        assert_eq!(config.name, "test");
        assert_eq!(config.command, "npx");
        assert_eq!(config.args.len(), 2);
        assert_eq!(config.env.get("NODE_ENV"), Some(&"production".to_string()));
        assert!(config.enabled);
    }

    #[test]
    fn test_connection_state_display() {
        assert_eq!(ConnectionState::Disconnected.to_string(), "disconnected");
        assert_eq!(ConnectionState::Connected.to_string(), "connected");
        assert_eq!(ConnectionState::Ready.to_string(), "ready");
        assert_eq!(ConnectionState::Closing.to_string(), "closing");
    }

    #[tokio::test]
    async fn test_mcp_client_new() {
        let client = McpClient::new();
        assert!(client.list_servers().await.is_empty());
    }
}
