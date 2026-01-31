//! MCP server discovery and management.
//!
//! This module provides `McpServerManager` for managing the lifecycle of
//! MCP servers, including starting, stopping, and registering tools.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::client::{McpClient, McpServerConfig};
use crate::error::McpError;
use crate::tools::register_mcp_tools;

/// Manages MCP server connections and tool registration.
pub struct McpServerManager {
    /// MCP client for managing connections.
    client: Arc<McpClient>,
    /// Server configurations.
    configs: RwLock<Vec<McpServerConfig>>,
}

impl McpServerManager {
    /// Create a new server manager.
    pub fn new() -> Self {
        Self {
            client: Arc::new(McpClient::new()),
            configs: RwLock::new(Vec::new()),
        }
    }

    /// Create a server manager with the given configurations.
    pub fn with_configs(configs: Vec<McpServerConfig>) -> Self {
        Self {
            client: Arc::new(McpClient::new()),
            configs: RwLock::new(configs),
        }
    }

    /// Get a reference to the MCP client.
    pub fn client(&self) -> Arc<McpClient> {
        Arc::clone(&self.client)
    }

    /// Add a server configuration.
    pub async fn add_config(&self, config: McpServerConfig) {
        let mut configs = self.configs.write().await;
        configs.push(config);
    }

    /// Remove a server configuration by name.
    pub async fn remove_config(&self, name: &str) -> bool {
        let mut configs = self.configs.write().await;
        let original_len = configs.len();
        configs.retain(|c| c.name != name);
        configs.len() < original_len
    }

    /// Get all server configurations.
    pub async fn list_configs(&self) -> Vec<McpServerConfig> {
        self.configs.read().await.clone()
    }

    /// Get a server configuration by name.
    pub async fn get_config(&self, name: &str) -> Option<McpServerConfig> {
        self.configs
            .read()
            .await
            .iter()
            .find(|c| c.name == name)
            .cloned()
    }

    /// Start all enabled servers.
    ///
    /// Servers that fail to start are logged but don't prevent other servers
    /// from starting.
    pub async fn start_all(&self) -> Result<(), McpError> {
        let configs = self.configs.read().await.clone();

        info!(count = configs.len(), "Starting MCP servers");

        let mut success_count = 0;
        let mut failure_count = 0;

        for config in configs {
            if !config.enabled {
                debug!(server = %config.name, "Skipping disabled server");
                continue;
            }

            match self.client.connect(config.clone()).await {
                Ok(connection) => {
                    // List tools after connecting
                    if let Err(e) = connection.list_tools().await {
                        warn!(
                            server = %config.name,
                            error = %e,
                            "Failed to list tools after connection"
                        );
                    }
                    success_count += 1;
                }
                Err(e) => {
                    error!(
                        server = %config.name,
                        error = %e,
                        "Failed to start server"
                    );
                    failure_count += 1;
                }
            }
        }

        info!(
            success = success_count,
            failed = failure_count,
            "MCP servers started"
        );

        Ok(())
    }

    /// Start a specific server by name.
    pub async fn start_server(&self, name: &str) -> Result<(), McpError> {
        let config = self
            .get_config(name)
            .await
            .ok_or_else(|| McpError::ServerNotFound(name.to_string()))?;

        let connection = self.client.connect(config).await?;

        // List tools after connecting
        connection.list_tools().await?;

        Ok(())
    }

    /// Stop all servers.
    pub async fn stop_all(&self) -> Result<(), McpError> {
        info!("Stopping all MCP servers");
        self.client.disconnect_all().await
    }

    /// Stop a specific server by name.
    pub async fn stop_server(&self, name: &str) -> Result<(), McpError> {
        self.client.disconnect(name).await
    }

    /// List all currently connected servers.
    pub async fn list_servers(&self) -> Vec<String> {
        self.client.list_servers().await
    }

    /// Check if a server is connected.
    pub async fn is_connected(&self, name: &str) -> bool {
        self.client.get(name).await.is_some()
    }

    /// Get server status.
    pub async fn server_status(&self, name: &str) -> ServerStatus {
        if let Some(connection) = self.client.get(name).await {
            let state = connection.state().await;
            let tool_count = connection.tools().await.len();
            ServerStatus::Connected { state: state.to_string(), tool_count }
        } else {
            ServerStatus::Disconnected
        }
    }

    /// Register all MCP tools with a tool registry.
    pub async fn register_tools(&self, registry: &mut agentik_tools::ToolRegistry) {
        register_mcp_tools(Arc::clone(&self.client), registry).await;
    }

    /// Refresh tools from all connected servers.
    pub async fn refresh_tools(&self) -> Result<(), McpError> {
        self.client.refresh_tools().await
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Status of an MCP server.
#[derive(Debug, Clone)]
pub enum ServerStatus {
    /// Server is disconnected.
    Disconnected,
    /// Server is connected.
    Connected {
        /// Connection state.
        state: String,
        /// Number of available tools.
        tool_count: usize,
    },
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "disconnected"),
            Self::Connected { state, tool_count } => {
                write!(f, "{} ({} tools)", state, tool_count)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_manager_new() {
        let manager = McpServerManager::new();
        assert!(manager.list_configs().await.is_empty());
        assert!(manager.list_servers().await.is_empty());
    }

    #[tokio::test]
    async fn test_add_and_remove_config() {
        let manager = McpServerManager::new();

        let config = McpServerConfig::new("test", "echo");
        manager.add_config(config).await;

        assert_eq!(manager.list_configs().await.len(), 1);
        assert!(manager.get_config("test").await.is_some());

        assert!(manager.remove_config("test").await);
        assert!(manager.list_configs().await.is_empty());

        // Removing again should return false
        assert!(!manager.remove_config("test").await);
    }

    #[tokio::test]
    async fn test_with_configs() {
        let configs = vec![
            McpServerConfig::new("server1", "cmd1"),
            McpServerConfig::new("server2", "cmd2"),
        ];

        let manager = McpServerManager::with_configs(configs);
        assert_eq!(manager.list_configs().await.len(), 2);
    }

    #[test]
    fn test_server_status_display() {
        let status = ServerStatus::Disconnected;
        assert_eq!(status.to_string(), "disconnected");

        let status = ServerStatus::Connected {
            state: "ready".to_string(),
            tool_count: 5,
        };
        assert_eq!(status.to_string(), "ready (5 tools)");
    }
}
